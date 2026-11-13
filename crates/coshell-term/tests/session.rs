use std::io::Cursor;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use coshell_term::alacritty_terminal::grid::Dimensions;
use coshell_term::alacritty_terminal::index::{Column, Line, Point};
use coshell_term::alacritty_terminal::term::cell::Flags;
use coshell_term::{GridSize, Session, TermEvent, Transport};
use futures::StreamExt;

#[derive(Default)]
struct Capture {
    written: Mutex<Vec<u8>>,
    resized: Mutex<Option<GridSize>>,
}
impl Transport for Capture {
    fn write(&self, bytes: &[u8]) {
        self.written.lock().unwrap().extend_from_slice(bytes);
    }
    fn resize(&self, size: GridSize) {
        *self.resized.lock().unwrap() = Some(size);
    }
}

fn session(input: &'static [u8]) -> (Session, Arc<Capture>) {
    let cap = Arc::new(Capture::default());
    let s = Session::new(cap.clone(), Box::new(Cursor::new(input)), GridSize::DEFAULT);
    (s, cap)
}

fn wait_exit(s: &mut Session) -> Vec<TermEvent> {
    let mut rx = s.take_events().unwrap();
    futures::executor::block_on(async {
        let mut seen = vec![];
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            match rx.next().await {
                Some(TermEvent::Exited) | None => break,
                Some(e) => seen.push(e),
            }
        }
        seen
    })
}

fn row_text(s: &Session, line: i32) -> String {
    let term = s.term().lock();
    let cols = term.columns();
    (0..cols)
        .map(|c| term.grid()[Point::new(Line(line), Column(c))].c)
        .collect::<String>()
        .trim_end()
        .to_string()
}

#[test]
fn parses_output_into_grid_and_signals_wakeup() {
    let (mut s, _) = session(b"hello\r\nworld");
    let ev = wait_exit(&mut s);
    assert!(ev.contains(&TermEvent::Wakeup));
    assert_eq!(row_text(&s, 0), "hello");
    assert_eq!(row_text(&s, 1), "world");
}

#[test]
fn hangul_occupies_two_cells() {
    let (mut s, _) = session("가a".as_bytes());
    wait_exit(&mut s);
    let term = s.term().lock();
    let c0 = &term.grid()[Point::new(Line(0), Column(0))];
    let c1 = &term.grid()[Point::new(Line(0), Column(1))];
    let c2 = &term.grid()[Point::new(Line(0), Column(2))];
    assert_eq!(c0.c, '가');
    assert!(c0.flags.contains(Flags::WIDE_CHAR));
    assert!(c1.flags.contains(Flags::WIDE_CHAR_SPACER));
    assert_eq!(c2.c, 'a');
}

#[test]
fn terminal_replies_go_back_to_transport() {
    // DSR cursor position request -> CPR reply on the transport.
    let (mut s, cap) = session(b"\x1b[6n");
    wait_exit(&mut s);
    assert_eq!(&*cap.written.lock().unwrap(), b"\x1b[1;1R");
}

#[test]
fn title_event_and_resize() {
    let (mut s, cap) = session(b"\x1b]0;prod-web-01\x07");
    let ev = wait_exit(&mut s);
    assert!(ev.contains(&TermEvent::Title("prod-web-01".into())));
    let size = GridSize { cols: 120, rows: 40, cell_width: 8, cell_height: 18 };
    s.resize(size);
    assert_eq!(*cap.resized.lock().unwrap(), Some(size));
    assert_eq!(s.term().lock().columns(), 120);
}

/// Parser throughput (run: cargo test --release -- --ignored --nocapture).
#[test]
#[ignore]
fn throughput_200k_lines() {
    let data: Vec<u8> = (1..=200_000).flat_map(|i| format!("\x1b[32m{i}\x1b[0m line of output 한글\r\n").into_bytes()).collect();
    let mb = data.len() as f64 / 1e6;
    let input: &'static [u8] = Box::leak(data.into_boxed_slice());
    let t = Instant::now();
    let (mut s, _) = session(input);
    wait_exit(&mut s);
    let secs = t.elapsed().as_secs_f64();
    println!("parsed {mb:.1} MB in {secs:.3}s = {:.0} MB/s", mb / secs);
}

#[test]
fn recent_text_skips_blank_tail_and_limits() {
    let (mut s, _) = session("l1\r\nl2 한글\r\nl3\r\n".as_bytes());
    wait_exit(&mut s);
    assert_eq!(s.recent_text(10), "l1\nl2 한글\nl3");
    assert_eq!(s.recent_text(2), "l2 한글\nl3");
}

#[test]
fn search_selects_matches_and_walks_both_ways() {
    let (mut s, _) = session(b"alpha one\r\nbeta\r\nAlpha two\r\n");
    wait_exit(&mut s);
    let start = |s: &Session| {
        let term = s.term().lock();
        term.selection.as_ref().and_then(|x| x.to_range(&term)).map(|r| r.start.line.0)
    };
    assert_eq!(s.selection_text(), None);
    // Nearest match above the bottom first, case-insensitive for lowercase queries.
    assert!(s.search("alpha", false));
    assert_eq!((s.selection_text().as_deref(), start(&s)), (Some("Alpha"), Some(2)));
    assert!(s.search("alpha", false));
    assert_eq!((s.selection_text().as_deref(), start(&s)), (Some("alpha"), Some(0)));
    assert!(s.search("alpha", true));
    assert_eq!(start(&s), Some(2));
    // An uppercase query is case-sensitive.
    s.clear_selection();
    assert!(s.search("Alpha", false));
    assert!(s.search("Alpha", false));
    assert_eq!(start(&s), Some(2));
    assert!(!s.search("nomatch", false));
    assert_eq!(s.selection_text(), None);
}

#[test]
fn configure_applies_scrollback_and_cursor_shape() {
    use coshell_term::alacritty_terminal::vte::ansi::CursorShape;
    let data: Vec<u8> = (0..60).flat_map(|i| format!("line {i}\r\n").into_bytes()).collect();
    let (mut s, _) = session(Box::leak(data.into_boxed_slice()));
    wait_exit(&mut s);
    assert!(s.term().lock().grid().history_size() > 5);
    s.configure(5, CursorShape::Beam);
    let term = s.term().lock();
    assert_eq!(term.grid().history_size(), 5);
    assert_eq!(term.renderable_content().cursor.shape, CursorShape::Beam);
}

#[test]
fn logical_lines_join_wrapped_rows() {
    let size = GridSize { cols: 10, rows: 5, ..GridSize::DEFAULT };
    let cap = Arc::new(Capture::default());
    let s = Session::new(cap, Box::new(Cursor::new(&b"$ echo 0123456789abcdef\r\nok\r\n$ "[..])), size);
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let lines = coshell_term::logical_lines(s.term(), 50);
        if lines.last().map(String::as_str) == Some("$") || Instant::now() > deadline {
            assert_eq!(lines, ["$ echo 0123456789abcdef", "ok", "$"]);
            assert_eq!(coshell_term::logical_lines(s.term(), 1), ["$"], "blank rows below do not count");
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
