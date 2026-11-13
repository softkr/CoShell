//! Transport-agnostic terminal core.
//!
//! A [`Session`] owns an `alacritty_terminal::Term` and feeds it bytes coming
//! from any [`Transport`] (local PTY today, SSH channel next). The UI layer only
//! reads the grid under the lock and writes input bytes back.

mod local;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{Config, Term};
use alacritty_terminal::vte::ansi::{CursorShape, CursorStyle, Processor};
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};

pub use alacritty_terminal;
pub use local::{LocalPty, local_shell_name};

/// Scrollback lines kept per session.
const SCROLLBACK_LINES: usize = 10_000;
/// Read buffer for the transport reader thread.
const READ_CHUNK: usize = 64 * 1024;

/// Terminal grid size, in cells and pixels per cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GridSize {
    pub cols: u16,
    pub rows: u16,
    pub cell_width: u16,
    pub cell_height: u16,
}

impl GridSize {
    pub const DEFAULT: Self = Self { cols: 80, rows: 24, cell_width: 8, cell_height: 16 };

    fn term_size(self) -> TermSize {
        TermSize::new(self.cols.max(1) as usize, self.rows.max(1) as usize)
    }
}

/// Events the UI cares about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TermEvent {
    /// Grid content changed; repaint.
    Wakeup,
    Title(String),
    Bell,
    /// Transport closed (shell exited or connection dropped).
    Exited,
}

/// A byte pipe to the remote/local shell.
pub trait Transport: Send + Sync + 'static {
    fn write(&self, bytes: &[u8]);
    fn resize(&self, size: GridSize);
}

/// Bridges alacritty's events to the UI channel and transport.
#[derive(Clone)]
pub struct Listener {
    events: UnboundedSender<TermEvent>,
    transport: Arc<dyn Transport>,
}

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        match event {
            // Terminal replies (DSR, DA, ...) must go back to the shell.
            Event::PtyWrite(text) => self.transport.write(text.as_bytes()),
            Event::Title(title) => {
                let _ = self.events.unbounded_send(TermEvent::Title(title));
            }
            Event::ResetTitle => {
                let _ = self.events.unbounded_send(TermEvent::Title(String::new()));
            }
            Event::Bell => {
                let _ = self.events.unbounded_send(TermEvent::Bell);
            }
            _ => {}
        }
    }
}

pub type SharedTerm = Arc<FairMutex<Term<Listener>>>;

/// The last `max_rows` rows with text (screen + scrollback) as lines, oldest first.
/// Rows the terminal wrapped are joined back into one line; the blank screen
/// area below the last text is skipped.
pub fn logical_lines(term: &SharedTerm, max_rows: usize) -> Vec<String> {
    use alacritty_terminal::grid::Dimensions;
    use alacritty_terminal::index::{Column, Line};
    use alacritty_terminal::term::cell::Flags;

    let term = term.lock();
    let grid = term.grid();
    let top = -(grid.history_size() as i32);
    let bottom = grid.screen_lines() as i32 - 1;
    let last_col = Column(grid.columns().saturating_sub(1));
    // (text, continues on the next row)
    let mut rows: Vec<(String, bool)> = Vec::new();
    for l in (top..=bottom).rev() {
        let row = &grid[Line(l)];
        let text: String = (0..grid.columns())
            .map(|c| &row[Column(c)])
            .filter(|cell| !cell.flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER))
            .map(|cell| cell.c)
            .collect();
        if rows.is_empty() && text.trim().is_empty() {
            continue; // blank screen area below the last text does not count
        }
        rows.push((text, row[last_col].flags.contains(Flags::WRAPLINE)));
        if rows.len() == max_rows {
            break;
        }
    }
    rows.reverse();
    let mut lines: Vec<String> = Vec::new();
    let mut joining = false;
    for (text, wraps) in rows {
        match lines.last_mut() {
            Some(prev) if joining => prev.push_str(&text),
            _ => lines.push(text),
        }
        if !wraps {
            if let Some(l) = lines.last_mut() {
                *l = l.trim_end().to_string();
            }
        }
        joining = wraps;
    }
    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    lines
}

/// One terminal session: emulator state + transport.
pub struct Session {
    term: SharedTerm,
    transport: Arc<dyn Transport>,
    events: Option<UnboundedReceiver<TermEvent>>,
    dirty: Arc<AtomicBool>,
    size: GridSize,
}

impl Session {
    /// Spawn the user's default shell on a local PTY.
    pub fn spawn_local(size: GridSize) -> anyhow::Result<Self> {
        let (pty, reader) = LocalPty::spawn(size)?;
        Ok(Self::new(Arc::new(pty), reader, size))
    }

    /// Wire a transport and its byte reader into a new session.
    /// The reader runs on its own thread until EOF/error.
    pub fn new(
        transport: Arc<dyn Transport>,
        mut reader: Box<dyn std::io::Read + Send>,
        size: GridSize,
    ) -> Self {
        let (tx, rx) = unbounded();
        let listener = Listener { events: tx.clone(), transport: transport.clone() };
        let config = Config { scrolling_history: SCROLLBACK_LINES, ..Config::default() };
        let term: SharedTerm =
            Arc::new(FairMutex::new(Term::new(config, &size.term_size(), listener)));
        let dirty = Arc::new(AtomicBool::new(false));

        let (t, d) = (term.clone(), dirty.clone());
        std::thread::Builder::new()
            .name("coshell-term-reader".into())
            .spawn(move || {
                let mut parser: Processor = Processor::new();
                let mut buf = vec![0u8; READ_CHUNK];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            parser.advance(&mut *t.lock(), &buf[..n]);
                            // Coalesce: only one pending Wakeup at a time.
                            if !d.swap(true, Ordering::AcqRel) {
                                let _ = tx.unbounded_send(TermEvent::Wakeup);
                            }
                        }
                    }
                }
                let _ = tx.unbounded_send(TermEvent::Exited);
            })
            .expect("spawn terminal reader thread");

        Self { term, transport, events: Some(rx), dirty, size }
    }

    pub fn term(&self) -> &SharedTerm {
        &self.term
    }

    /// Take the event stream (once). The UI drives repaint from it.
    pub fn take_events(&mut self) -> Option<UnboundedReceiver<TermEvent>> {
        self.events.take()
    }

    /// Mark the latest Wakeup as consumed so the reader may send another.
    pub fn clear_dirty(&self) {
        self.dirty.store(false, Ordering::Release);
    }

    pub fn write(&self, bytes: &[u8]) {
        if !bytes.is_empty() {
            self.transport.write(bytes);
        }
    }

    pub fn size(&self) -> GridSize {
        self.size
    }

    /// Last `max_lines` non-trailing-blank lines (scrollback + screen), plain text.
    pub fn recent_text(&self, max_lines: usize) -> String {
        use alacritty_terminal::grid::Dimensions;
        use alacritty_terminal::index::{Column, Line};
        use alacritty_terminal::term::cell::Flags;

        let term = self.term.lock();
        let grid = term.grid();
        let top = -(grid.history_size() as i32);
        let bottom = grid.screen_lines() as i32 - 1;
        let mut lines: Vec<String> = Vec::new();
        for l in (top..=bottom).rev() {
            let row = &grid[Line(l)];
            let text: String = (0..grid.columns())
                .map(|c| &row[Column(c)])
                .filter(|cell| !cell.flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER))
                .map(|cell| cell.c)
                .collect();
            let text = text.trim_end().to_string();
            if lines.is_empty() && text.is_empty() {
                continue; // blank area below the prompt
            }
            lines.push(text);
            if lines.len() == max_lines {
                break;
            }
        }
        lines.reverse();
        lines.join("\n")
    }

    /// Apply user preferences: scrollback length and the default cursor shape.
    pub fn configure(&self, scrollback: usize, cursor: CursorShape) {
        let config = Config {
            scrolling_history: scrollback,
            default_cursor_style: CursorStyle { shape: cursor, blinking: false },
            ..Config::default()
        };
        self.term.lock().set_options(config);
    }

    /// Selected text, if any.
    pub fn selection_text(&self) -> Option<String> {
        self.term.lock().selection_to_string().filter(|t| !t.is_empty())
    }

    pub fn clear_selection(&self) {
        self.term.lock().selection = None;
    }

    /// Select the next match of `pattern` (regex; smart case) and scroll it
    /// into view. Starts from the current selection, else from the bottom of
    /// the view going up. `forward` = towards newer output. False = no match.
    pub fn search(&self, pattern: &str, forward: bool) -> bool {
        use alacritty_terminal::grid::Dimensions;
        use alacritty_terminal::index::{Boundary, Column, Direction, Line, Point, Side};
        use alacritty_terminal::selection::{Selection, SelectionType};
        use alacritty_terminal::term::search::RegexSearch;

        let Ok(mut regex) = RegexSearch::new(pattern) else { return false };
        let mut term = self.term.lock();
        let current = term.selection.as_ref().and_then(|s| s.to_range(&term)).map(|r| r.start);
        let (origin, direction) = match (current, forward) {
            (Some(p), true) => (p.add(&*term, Boundary::None, 1), Direction::Right),
            (Some(p), false) => (p.sub(&*term, Boundary::None, 1), Direction::Left),
            (None, _) => {
                let offset = term.grid().display_offset() as i32;
                let bottom = Line(term.screen_lines() as i32 - 1 - offset);
                (Point::new(bottom, Column(term.columns() - 1)), Direction::Left)
            }
        };
        let Some(found) = term.search_next(&mut regex, origin, direction, Side::Left, None) else {
            term.selection = None;
            return false;
        };
        let mut selection = Selection::new(SelectionType::Simple, *found.start(), Side::Left);
        selection.update(*found.end(), Side::Right);
        term.selection = Some(selection);
        term.scroll_to_point(*found.start());
        true
    }

    pub fn resize(&mut self, size: GridSize) {
        if size == self.size || size.cols == 0 || size.rows == 0 {
            return;
        }
        self.size = size;
        self.term.lock().resize(size.term_size());
        self.transport.resize(size);
    }
}
