//! iced terminal widget on top of `coshell_term::Session`.
//!
//! - Renders the alacritty grid with `fill_quad` / `fill_text`.
//! - ASCII runs are drawn as one text batch; wide / non-ASCII cells are drawn
//!   per cell at their grid column so CJK fallback glyphs never drift.
//! - IME: requests `Purpose::Terminal`, draws the pre-edit on the spot,
//!   sends committed text to the shell.

pub mod keys;
pub mod link;

use coshell_term::alacritty_terminal::grid::{Dimensions, Scroll};
use coshell_term::alacritty_terminal::index::{Column, Line, Point as GridPoint, Side};
use coshell_term::alacritty_terminal::selection::{Selection, SelectionType};
use coshell_term::alacritty_terminal::term::TermMode;
use coshell_term::alacritty_terminal::term::cell::Flags;
use coshell_term::alacritty_terminal::term::color::Colors;
use coshell_term::alacritty_terminal::vte::ansi::{Color as AnsiColor, CursorShape, NamedColor};
use coshell_term::{GridSize, SharedTerm};
use iced::advanced::input_method::{self, InputMethod, Purpose};
use iced::advanced::layout::{self, Layout};
use iced::advanced::mouse::click::{Click, Kind as ClickKind};
use iced::advanced::renderer::{self, Quad};
use iced::advanced::text::{self, LineHeight, Paragraph, Shaping, Text, Wrapping};
use iced::advanced::widget::{Tree, tree};
use iced::advanced::{Clipboard, Shell, Widget, clipboard};
use iced::font::Weight;
use iced::keyboard;
use iced::{
    Border, Color, Element, Event, Font, Length, Pixels, Point, Rectangle, Shadow, Size, Vector,
    alignment, mouse, window,
};
use unicode_width::UnicodeWidthStr;

use crate::theme::TermPalette;

/// Line height as a multiple of font size.
const LINE_HEIGHT_FACTOR: f32 = 1.4;
/// Inner padding around the grid, px.
const PAD: f32 = 10.0;
/// Characters used to measure the monospace cell width.
const MEASURE_SAMPLE: &str = "MMMMMMMMMM";
/// Beam/underline cursor thickness, px.
const CURSOR_THICKNESS: f32 = 2.0;
/// Pixels per scroll line for precise (trackpad) scrolling.
const PIXELS_PER_LINE: f32 = 18.0;
/// Scrollback position indicator at the right edge, px.
const SCROLL_THUMB_W: f32 = 4.0;
const SCROLL_THUMB_MIN_H: f32 = 24.0;
const SCROLL_THUMB_INSET: f32 = 3.0;
const SCROLL_THUMB_ALPHA: f32 = 0.22;
const SCROLL_THUMB_ALPHA_SCROLLED: f32 = 0.5;
/// Wheel reports sent to a mouse-aware program per scroll event.
const MAX_WHEEL_REPORTS: u32 = 10;

pub struct TerminalView<'a, Message> {
    term: &'a SharedTerm,
    size: GridSize,
    palette: &'a TermPalette,
    font: Font,
    font_size: f32,
    on_input: Box<dyn Fn(Vec<u8>) -> Message + 'a>,
    on_resize: Box<dyn Fn(GridSize) -> Message + 'a>,
    /// ⌘+click on a web link.
    on_link: Option<Box<dyn Fn(String) -> Message + 'a>>,
    /// False while an overlay (palette, dialog) owns the keyboard.
    active: bool,
    /// App-driven focus change: applied when the generation changes.
    focus_gen: u64,
    focus_want: bool,
    option_as_meta: bool,
}

impl<'a, Message> TerminalView<'a, Message> {
    pub fn new(
        term: &'a SharedTerm,
        size: GridSize,
        palette: &'a TermPalette,
        on_input: impl Fn(Vec<u8>) -> Message + 'a,
        on_resize: impl Fn(GridSize) -> Message + 'a,
    ) -> Self {
        Self {
            term,
            size,
            palette,
            font: Font::MONOSPACE,
            font_size: 13.0,
            on_input: Box::new(on_input),
            on_resize: Box::new(on_resize),
            on_link: None,
            active: true,
            focus_gen: 0,
            focus_want: true,
            option_as_meta: true,
        }
    }

    pub fn on_link(mut self, f: impl Fn(String) -> Message + 'a) -> Self {
        self.on_link = Some(Box::new(f));
        self
    }

    /// Option/Alt sends ESC-prefixed keys instead of the layout's special characters.
    pub fn option_as_meta(mut self, on: bool) -> Self {
        self.option_as_meta = on;
        self
    }

    pub fn focus_request(mut self, generation: u64, focused: bool) -> Self {
        self.focus_gen = generation;
        self.focus_want = focused;
        self
    }

    pub fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }

    pub fn font(mut self, font: Font, size: f32) -> Self {
        self.font = font;
        self.font_size = size;
        self
    }
}

#[derive(Clone, Copy, PartialEq)]
struct Metrics {
    cell_w: f32,
    cell_h: f32,
    font: Font,
    font_size: f32,
}

struct State {
    focused: bool,
    window_focused: bool,
    preedit: Option<String>,
    metrics: Option<Metrics>,
    scroll_px: f32,
    last_sent: Option<GridSize>,
    focus_seen: u64,
    /// Left button is held after a press inside the grid: extend the selection.
    selecting: bool,
    last_click: Option<Click>,
    mods: keyboard::Modifiers,
    /// Button code being reported to a mouse-aware program (vim, tmux, htop).
    reporting: Option<u8>,
    /// Last cell reported, to send motion once per cell.
    reported_cell: Option<(usize, usize)>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            focused: true,
            window_focused: true,
            preedit: None,
            metrics: None,
            scroll_px: 0.0,
            last_sent: None,
            focus_seen: 0,
            selecting: false,
            last_click: None,
            mods: keyboard::Modifiers::default(),
            reporting: None,
            reported_cell: None,
        }
    }
}

fn measure<R: text::Renderer<Font = Font>>(font: Font, font_size: f32) -> Metrics {
    let p = R::Paragraph::with_text(Text {
        content: MEASURE_SAMPLE,
        bounds: Size::INFINITE,
        size: Pixels(font_size),
        line_height: LineHeight::Relative(1.0),
        font,
        align_x: text::Alignment::Left,
        align_y: alignment::Vertical::Top,
        shaping: Shaping::Basic,
        wrapping: Wrapping::None,
    });
    Metrics {
        cell_w: p.min_width() / MEASURE_SAMPLE.len() as f32,
        cell_h: (font_size * LINE_HEIGHT_FACTOR).round(),
        font,
        font_size,
    }
}

fn grid_for(bounds: Rectangle, m: &Metrics) -> GridSize {
    let cols = ((bounds.width - PAD * 2.0) / m.cell_w).floor().max(1.0);
    let rows = ((bounds.height - PAD * 2.0) / m.cell_h).floor().max(1.0);
    GridSize {
        cols: cols as u16,
        rows: rows as u16,
        cell_width: m.cell_w.round() as u16,
        cell_height: m.cell_h as u16,
    }
}

// ---------------------------------------------------------------- colours

fn rgb(c: coshell_term::alacritty_terminal::vte::ansi::Rgb) -> Color {
    Color::from_rgb8(c.r, c.g, c.b)
}

fn dim(c: Color, bg: Color) -> Color {
    Color { r: c.r * 0.66 + bg.r * 0.34, g: c.g * 0.66 + bg.g * 0.34, b: c.b * 0.66 + bg.b * 0.34, a: c.a }
}

fn indexed(i: usize, p: &TermPalette, colors: &Colors) -> Color {
    if let Some(c) = colors[i] {
        return rgb(c);
    }
    match i {
        0..=15 => p.ansi[i],
        16..=231 => {
            let i = i - 16;
            let level = |v: usize| if v == 0 { 0 } else { (v * 40 + 55) as u8 };
            Color::from_rgb8(level(i / 36), level((i / 6) % 6), level(i % 6))
        }
        232..=255 => {
            let v = (8 + (i - 232) * 10) as u8;
            Color::from_rgb8(v, v, v)
        }
        _ => p.fg,
    }
}

fn resolve(c: AnsiColor, p: &TermPalette, colors: &Colors) -> Color {
    match c {
        AnsiColor::Spec(c) => rgb(c),
        AnsiColor::Indexed(i) => indexed(i as usize, p, colors),
        AnsiColor::Named(n) => {
            if let Some(c) = colors[n] {
                return rgb(c);
            }
            match n {
                NamedColor::Foreground | NamedColor::BrightForeground => p.fg,
                NamedColor::Background => p.bg,
                NamedColor::Cursor => p.cursor,
                NamedColor::DimForeground => dim(p.fg, p.bg),
                n if (n as usize) < 16 => p.ansi[n as usize],
                n => {
                    let base = n as usize - NamedColor::DimBlack as usize;
                    dim(p.ansi[base.min(7)], p.bg)
                }
            }
        }
    }
}

// ---------------------------------------------------------------- frame

struct TextRun {
    pos: Point,
    text: String,
    color: Color,
    font: Font,
    width: f32,
    advanced: bool,
}

struct CursorDraw {
    rect: Rectangle,
    shape: CursorShape,
    ch: Option<(String, Font, Color, f32)>,
}

#[derive(Default)]
struct Frame {
    rects: Vec<(Rectangle, Color)>,
    texts: Vec<TextRun>,
    cursor: Option<CursorDraw>,
    /// Scrollback indicator: (top, height) as fractions of the view, and whether the view is scrolled.
    scroll: Option<(f32, f32, bool)>,
}

/// Snapshot the grid into draw commands (holds the term lock only here).
fn build_frame(term: &SharedTerm, origin: Point, m: &Metrics, p: &TermPalette) -> Frame {
    let term = term.lock();
    let content = term.renderable_content();
    let offset = content.display_offset as i32;
    let colors = content.colors;
    let selection = content.selection;
    let mut f = Frame::default();
    let history = term.grid().history_size();
    if history > 0 {
        let total = (history + term.screen_lines()) as f32;
        let top = (history - content.display_offset.min(history)) as f32 / total;
        f.scroll = Some((top, term.screen_lines() as f32 / total, content.display_offset > 0));
    }
    let cell_rect = |line: i32, col: usize, w: usize| Rectangle {
        x: origin.x + col as f32 * m.cell_w,
        y: origin.y + line as f32 * m.cell_h,
        width: w as f32 * m.cell_w,
        height: m.cell_h,
    };

    // Current ASCII batch: (line, start col, text, colour, font).
    let mut batch: Option<(i32, usize, String, Color, Font)> = None;
    let flush = |b: &mut Option<(i32, usize, String, Color, Font)>, f: &mut Frame| {
        if let Some((line, col, text, color, font)) = b.take() {
            let r = cell_rect(line, col, text.len());
            if !text.trim().is_empty() {
                f.texts.push(TextRun { pos: r.position(), width: r.width, text, color, font, advanced: false });
            }
        }
    };

    let cursor_point = content.cursor.point;
    let cursor_line = cursor_point.line.0 + offset;
    let mut cursor_glyph = None;

    for item in content.display_iter {
        let line = item.point.line.0 + offset;
        let col = item.point.column.0;
        let cell = item.cell;
        if cell.flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
            continue;
        }
        let mut fg = resolve(cell.fg, p, colors);
        let mut bg = resolve(cell.bg, p, colors);
        if cell.flags.contains(Flags::INVERSE) {
            std::mem::swap(&mut fg, &mut bg);
        }
        if cell.flags.contains(Flags::DIM) {
            fg = dim(fg, p.bg);
        }
        if selection.is_some_and(|r| r.contains(item.point)) {
            bg = p.selection;
        }
        let width = if cell.flags.contains(Flags::WIDE_CHAR) { 2 } else { 1 };
        let font = if cell.flags.contains(Flags::BOLD) { Font { weight: Weight::Bold, ..m.font } } else { m.font };

        if bg != p.bg {
            f.rects.push((cell_rect(line, col, width), bg));
        }
        if cell.flags.intersects(Flags::UNDERLINE | Flags::DOUBLE_UNDERLINE | Flags::UNDERCURL) {
            let r = cell_rect(line, col, width);
            f.rects.push((Rectangle { y: r.y + r.height - 2.0, height: 1.0, ..r }, fg));
        }
        if line == cursor_line && col == cursor_point.column.0 {
            let mut s = String::from(cell.c);
            if let Some(z) = cell.zerowidth() {
                s.extend(z);
            }
            cursor_glyph = Some((s, font, bg, width));
        }

        let ch = cell.c;
        let hidden = cell.flags.contains(Flags::HIDDEN);
        let ascii = ch.is_ascii() && width == 1 && cell.zerowidth().is_none();
        if ascii {
            let c = if hidden || ch == '\t' { ' ' } else { ch };
            match &mut batch {
                Some((bl, bc, text, bcolor, bfont))
                    if *bl == line && *bc + text.len() == col && *bcolor == fg && *bfont == font =>
                {
                    text.push(c)
                }
                _ => {
                    flush(&mut batch, &mut f);
                    batch = Some((line, col, c.to_string(), fg, font));
                }
            }
        } else {
            flush(&mut batch, &mut f);
            if !hidden {
                let mut s = String::from(ch);
                if let Some(z) = cell.zerowidth() {
                    s.extend(z);
                }
                let r = cell_rect(line, col, width);
                f.texts.push(TextRun { pos: r.position(), width: r.width, text: s, color: fg, font, advanced: true });
            }
        }
    }
    flush(&mut batch, &mut f);

    let shape = content.cursor.shape;
    if shape != CursorShape::Hidden && (0..term.screen_lines_i32()).contains(&cursor_line) {
        let w = cursor_glyph.as_ref().map_or(1, |g| g.3);
        f.cursor = Some(CursorDraw {
            rect: cell_rect(cursor_line, cursor_point.column.0, w),
            shape,
            ch: cursor_glyph.map(|(s, font, bg, w)| (s, font, if bg == p.bg { p.bg } else { bg }, w as f32)),
        });
    }
    f
}

/// Small extension: screen lines as i32 for range checks.
trait ScreenLines {
    fn screen_lines_i32(&self) -> i32;
}
impl<T> ScreenLines for coshell_term::alacritty_terminal::Term<T> {
    fn screen_lines_i32(&self) -> i32 {
        use coshell_term::alacritty_terminal::grid::Dimensions;
        self.screen_lines() as i32
    }
}

/// Bytes for pasted text; `bracketed` when the program asked for bracketed paste.
pub fn paste_bytes(text: &str, bracketed: bool) -> Vec<u8> {
    // Never let pasted content close the bracket early.
    let body = text.replace("\x1b[201~", "").replace("\r\n", "\r").replace('\n', "\r");
    if bracketed { format!("\x1b[200~{body}\x1b[201~") } else { body }.into_bytes()
}

fn text_of(content: String, run_width: f32, m: &Metrics, font: Font, advanced: bool) -> Text<String, Font> {
    Text {
        content,
        bounds: Size::new(run_width + m.cell_w, m.cell_h),
        size: Pixels(m.font_size),
        line_height: LineHeight::Absolute(Pixels(m.cell_h)),
        font,
        align_x: text::Alignment::Left,
        align_y: alignment::Vertical::Top,
        shaping: if advanced { Shaping::Advanced } else { Shaping::Basic },
        wrapping: Wrapping::None,
    }
}

fn fill<R: renderer::Renderer>(r: &mut R, bounds: Rectangle, color: Color) {
    r.fill_quad(Quad { bounds, border: Border::default(), shadow: Shadow::default(), snap: true }, color);
}

// ---------------------------------------------------------------- widget

impl<Message, Theme, R> Widget<Message, Theme, R> for TerminalView<'_, Message>
where
    R: text::Renderer<Font = Font>,
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Fill)
    }

    fn layout(&mut self, tree: &mut Tree, _renderer: &R, limits: &layout::Limits) -> layout::Node {
        let state = tree.state.downcast_mut::<State>();
        let stale = state.metrics.is_none_or(|m| m.font != self.font || m.font_size != self.font_size);
        if stale {
            state.metrics = Some(measure::<R>(self.font, self.font_size));
        }
        layout::Node::new(limits.max())
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _renderer: &R,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        _viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_mut::<State>();
        if state.focus_seen != self.focus_gen {
            state.focus_seen = self.focus_gen;
            state.focused = self.focus_want;
            shell.request_redraw();
        }
        let bounds = layout.bounds();
        let Some(m) = state.metrics else { return };

        match event {
            Event::Window(window::Event::RedrawRequested(_)) => {
                let want = grid_for(bounds, &m);
                if want != self.size && state.last_sent != Some(want) {
                    state.last_sent = Some(want);
                    shell.publish((self.on_resize)(want));
                }
                if self.active && state.focused && state.window_focused {
                    let c = self.cursor_rect(bounds, &m);
                    shell.request_input_method(&InputMethod::<&str>::Enabled {
                        cursor: c,
                        purpose: Purpose::Terminal,
                        preedit: None,
                    });
                }
            }
            Event::Window(window::Event::Focused) => state.window_focused = true,
            Event::Window(window::Event::Unfocused) => {
                state.window_focused = false;
                shell.request_redraw();
            }
            Event::Keyboard(keyboard::Event::ModifiersChanged(mods)) => state.mods = *mods,
            Event::Mouse(mouse::Event::ButtonPressed(button)) => {
                let over = cursor.position_over(bounds);
                if *button == mouse::Button::Left && over.is_some() != state.focused {
                    state.focused = over.is_some();
                    shell.request_redraw();
                }
                let Some(pos) = over else { return };
                if *button == mouse::Button::Left
                    && state.mods.command()
                    && let Some(on_link) = &self.on_link
                    && let Some(url) = self.link_at(pos, bounds, &m)
                {
                    shell.publish(on_link(url));
                    shell.capture_event();
                    return;
                }
                let code = match button {
                    mouse::Button::Left => Some(keys::MOUSE_LEFT),
                    mouse::Button::Middle => Some(keys::MOUSE_MIDDLE),
                    mouse::Button::Right => Some(keys::MOUSE_RIGHT),
                    _ => None,
                };
                // Mouse-aware programs get the click; Shift keeps it for local selection.
                if let Some(code) = code
                    && self.mouse_mode(state).is_some()
                {
                    self.report(code, pos, bounds, &m, true, state, shell);
                    state.reporting = Some(code);
                    shell.capture_event();
                    return;
                }
                if *button == mouse::Button::Left {
                    // Click starts a selection: drag = characters, double = word, triple = line.
                    let click = Click::new(pos, mouse::Button::Left, state.last_click);
                    let ty = match click.kind() {
                        ClickKind::Single => SelectionType::Simple,
                        ClickKind::Double => SelectionType::Semantic,
                        ClickKind::Triple => SelectionType::Lines,
                    };
                    state.last_click = Some(click);
                    let (point, side) = self.grid_point(pos, bounds, &m);
                    self.term.lock().selection = Some(Selection::new(ty, point, side));
                    state.selecting = true;
                    shell.request_redraw();
                }
            }
            Event::Mouse(mouse::Event::CursorMoved { position }) if state.selecting => {
                let (point, side) = self.grid_point(*position, bounds, &m);
                if let Some(s) = &mut self.term.lock().selection {
                    s.update(point, side);
                }
                shell.request_redraw();
            }
            Event::Mouse(mouse::Event::ButtonReleased(_)) if state.reporting.is_some() => {
                if let (Some(code), Some(pos)) = (state.reporting.take(), cursor.position()) {
                    self.report(code, pos, bounds, &m, false, state, shell);
                }
            }
            Event::Mouse(mouse::Event::CursorMoved { position }) if cursor.is_over(bounds) => {
                let Some(mode) = self.mouse_mode(state) else { return };
                let code = match state.reporting {
                    Some(code) if mode.intersects(TermMode::MOUSE_DRAG | TermMode::MOUSE_MOTION) => code + keys::MOUSE_MOTION,
                    None if mode.contains(TermMode::MOUSE_MOTION) => keys::MOUSE_NONE + keys::MOUSE_MOTION,
                    _ => return,
                };
                if state.reported_cell != Some(self.cell_at(*position, bounds, &m)) {
                    self.report(code, *position, bounds, &m, true, state, shell);
                }
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) if state.selecting => {
                state.selecting = false;
                let mut term = self.term.lock();
                if term.selection.as_ref().is_some_and(Selection::is_empty) {
                    term.selection = None;
                }
                shell.request_redraw();
            }
            Event::Mouse(mouse::Event::WheelScrolled { delta }) if cursor.is_over(bounds) => {
                let lines = match delta {
                    mouse::ScrollDelta::Lines { y, .. } => *y,
                    mouse::ScrollDelta::Pixels { y, .. } => {
                        state.scroll_px += *y;
                        let l = (state.scroll_px / PIXELS_PER_LINE).trunc();
                        state.scroll_px -= l * PIXELS_PER_LINE;
                        l
                    }
                };
                let n = lines as i32;
                if n != 0 {
                    match (self.mouse_mode(state), cursor.position()) {
                        (Some(_), Some(pos)) => {
                            let code = if n > 0 { keys::MOUSE_WHEEL_UP } else { keys::MOUSE_WHEEL_DOWN };
                            for _ in 0..n.unsigned_abs().min(MAX_WHEEL_REPORTS) {
                                self.report(code, pos, bounds, &m, true, state, shell);
                            }
                        }
                        _ => self.scroll(n, shell),
                    }
                    shell.capture_event();
                }
            }
            Event::InputMethod(ime) if self.active && state.focused => match ime {
                input_method::Event::Opened | input_method::Event::Closed => {
                    state.preedit = None;
                    shell.request_redraw();
                }
                input_method::Event::Preedit(content, _) => {
                    state.preedit = (!content.is_empty()).then(|| content.clone());
                    shell.request_redraw();
                }
                input_method::Event::Commit(text) => {
                    state.preedit = None;
                    self.send(text.as_bytes().to_vec(), shell);
                    shell.capture_event();
                }
            },
            Event::Keyboard(keyboard::Event::KeyPressed { key, text, modifiers, physical_key, .. })
                if self.active && state.focused =>
            {
                // While composing, keys belong to the IME.
                if state.preedit.is_some() {
                    return;
                }
                let latin = key.to_latin(*physical_key);
                // Copy: ⌘C on macOS; Ctrl+Shift+C elsewhere (Ctrl+C stays SIGINT).
                if modifiers.command() && latin == Some('c') && (cfg!(target_os = "macos") || modifiers.shift()) {
                    let selected = self.term.lock().selection_to_string().filter(|t| !t.is_empty());
                    if let Some(t) = selected {
                        clipboard.write(clipboard::Kind::Standard, t);
                    }
                    shell.capture_event();
                    return;
                }
                if modifiers.command() && latin == Some('v') {
                    if let Some(t) = clipboard.read(clipboard::Kind::Standard) {
                        self.paste(&t, shell);
                    }
                    shell.capture_event();
                    return;
                }
                let app_cursor = self.term.lock().mode().contains(TermMode::APP_CURSOR);
                let input = keys::KeyInput {
                    key,
                    latin,
                    text: text.as_deref(),
                    mods: *modifiers,
                    app_cursor,
                    option_as_meta: self.option_as_meta,
                };
                if let Some(bytes) = keys::encode(&input) {
                    self.send(bytes, shell);
                    shell.capture_event();
                }
            }
            _ => {}
        }
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut R,
        _theme: &Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_ref::<State>();
        let Some(m) = state.metrics else { return };
        let bounds = layout.bounds();
        let p = self.palette;
        fill(renderer, bounds, p.bg);

        let origin = bounds.position() + Vector::new(PAD, PAD);
        let frame = build_frame(self.term, origin, &m, p);

        for (r, c) in &frame.rects {
            fill(renderer, *r, *c);
        }
        for t in frame.texts {
            renderer.fill_text(text_of(t.text, t.width, &m, t.font, t.advanced), t.pos, t.color, bounds);
        }

        if let Some((top, height, scrolled)) = frame.scroll {
            let h = (bounds.height * height).max(SCROLL_THUMB_MIN_H).min(bounds.height);
            let thumb = Rectangle {
                x: bounds.x + bounds.width - SCROLL_THUMB_W - SCROLL_THUMB_INSET,
                y: bounds.y + (bounds.height * top).min(bounds.height - h),
                width: SCROLL_THUMB_W,
                height: h,
            };
            let alpha = if scrolled { SCROLL_THUMB_ALPHA_SCROLLED } else { SCROLL_THUMB_ALPHA };
            fill(renderer, thumb, Color { a: alpha, ..p.fg });
        }

        let active = self.active && state.focused && state.window_focused;
        if let Some(cur) = &frame.cursor {
            if let Some(pre) = &state.preedit {
                self.draw_preedit(renderer, pre, cur.rect.position(), &m, bounds);
            } else {
                self.draw_cursor(renderer, cur, active, &m, bounds);
            }
        }
    }

    fn mouse_interaction(
        &self,
        _tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _viewport: &Rectangle,
        _renderer: &R,
    ) -> mouse::Interaction {
        if cursor.is_over(layout.bounds()) { mouse::Interaction::Text } else { mouse::Interaction::default() }
    }
}

impl<Message> TerminalView<'_, Message> {
    fn send(&self, bytes: Vec<u8>, shell: &mut Shell<'_, Message>) {
        let mut term = self.term.lock();
        term.scroll_display(Scroll::Bottom);
        term.selection = None;
        drop(term);
        shell.publish((self.on_input)(bytes));
    }

    fn paste(&self, text: &str, shell: &mut Shell<'_, Message>) {
        let bracketed = self.term.lock().mode().contains(TermMode::BRACKETED_PASTE);
        self.send(paste_bytes(text, bracketed), shell);
    }

    fn scroll(&self, lines: i32, shell: &mut Shell<'_, Message>) {
        let mut term = self.term.lock();
        let mode = *term.mode();
        if mode.contains(TermMode::ALT_SCREEN) && mode.contains(TermMode::ALTERNATE_SCROLL) {
            // Full-screen apps (less, vim): translate wheel into arrow keys.
            let seq: &[u8] = if lines > 0 { b"\x1bOA" } else { b"\x1bOB" };
            drop(term);
            shell.publish((self.on_input)(seq.repeat(lines.unsigned_abs() as usize)));
        } else {
            term.scroll_display(Scroll::Delta(lines));
            shell.request_redraw();
        }
    }

    /// Mouse reporting mode of the running program, unless Shift asks for local selection.
    fn mouse_mode(&self, state: &State) -> Option<TermMode> {
        let mode = *self.term.lock().mode();
        (mode.intersects(TermMode::MOUSE_MODE) && !state.mods.shift()).then_some(mode)
    }

    /// Visible (column, row) under a window position, clamped to the grid.
    fn cell_at(&self, pos: Point, bounds: Rectangle, m: &Metrics) -> (usize, usize) {
        let (point, _) = self.grid_point(pos, bounds, m);
        let offset = self.term.lock().grid().display_offset() as i32;
        (point.column.0, (point.line.0 + offset).max(0) as usize)
    }

    /// Send one mouse report for the cell under `pos`.
    #[allow(clippy::too_many_arguments)]
    fn report(&self, code: u8, pos: Point, bounds: Rectangle, m: &Metrics, pressed: bool, state: &mut State, shell: &mut Shell<'_, Message>) {
        let (col, row) = self.cell_at(pos, bounds, m);
        state.reported_cell = Some((col, row));
        let sgr = self.term.lock().mode().contains(TermMode::SGR_MOUSE);
        if let Some(bytes) = keys::mouse_report(code + keys::mouse_mods(state.mods), col, row, pressed, sgr) {
            shell.publish((self.on_input)(bytes));
        }
    }

    /// Web link under a window position.
    fn link_at(&self, pos: Point, bounds: Rectangle, m: &Metrics) -> Option<String> {
        let (point, _) = self.grid_point(pos, bounds, m);
        let term = self.term.lock();
        let row = &term.grid()[point.line];
        let mut text = String::new();
        let mut at = 0;
        for c in 0..term.columns() {
            let cell = &row[Column(c)];
            if cell.flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
                continue;
            }
            if c <= point.column.0 {
                at = text.len();
            }
            text.push(cell.c);
        }
        link::url_at(&text, at).map(str::to_string)
    }

    /// Grid cell (and which half of it) under a window position, clamped to the grid.
    fn grid_point(&self, pos: Point, bounds: Rectangle, m: &Metrics) -> (GridPoint, Side) {
        let term = self.term.lock();
        let x = ((pos.x - bounds.x - PAD) / m.cell_w).max(0.0);
        let y = ((pos.y - bounds.y - PAD) / m.cell_h).max(0.0);
        let last_col = term.columns().saturating_sub(1);
        let col = (x as usize).min(last_col);
        let row = (y as usize).min(term.screen_lines().saturating_sub(1));
        let side = if x as usize > last_col || x.fract() >= 0.5 { Side::Right } else { Side::Left };
        let offset = term.grid().display_offset() as i32;
        (GridPoint::new(Line(row as i32 - offset), Column(col)), side)
    }

    fn cursor_rect(&self, bounds: Rectangle, m: &Metrics) -> Rectangle {
        let term = self.term.lock();
        let c = term.grid().cursor.point;
        Rectangle {
            x: bounds.x + PAD + c.column.0 as f32 * m.cell_w,
            y: bounds.y + PAD + c.line.0 as f32 * m.cell_h,
            width: m.cell_w,
            height: m.cell_h,
        }
    }

    fn draw_cursor<R: text::Renderer<Font = Font>>(
        &self,
        r: &mut R,
        cur: &CursorDraw,
        active: bool,
        m: &Metrics,
        clip: Rectangle,
    ) {
        let color = self.palette.cursor;
        let rect = cur.rect;
        if !active {
            r.fill_quad(
                Quad {
                    bounds: rect,
                    border: Border { color, width: 1.0, radius: 0.0.into() },
                    shadow: Shadow::default(),
                    snap: true,
                },
                Color::TRANSPARENT,
            );
            return;
        }
        match cur.shape {
            CursorShape::Beam => fill(r, Rectangle { width: CURSOR_THICKNESS, ..rect }, color),
            CursorShape::Underline => {
                fill(r, Rectangle { y: rect.y + rect.height - CURSOR_THICKNESS, height: CURSOR_THICKNESS, ..rect }, color)
            }
            _ => {
                fill(r, rect, color);
                if let Some((s, font, bg, _)) = &cur.ch
                    && !s.trim().is_empty()
                {
                    r.fill_text(text_of(s.clone(), rect.width, m, *font, true), rect.position(), *bg, clip);
                }
            }
        }
    }

    /// On-the-spot IME pre-edit (e.g. 한글 조합 중 글자) at the cursor.
    fn draw_preedit<R: text::Renderer<Font = Font>>(
        &self,
        r: &mut R,
        pre: &str,
        at: Point,
        m: &Metrics,
        clip: Rectangle,
    ) {
        let w = pre.width().max(1) as f32 * m.cell_w;
        let rect = Rectangle { x: at.x, y: at.y, width: w, height: m.cell_h };
        fill(r, rect, self.palette.bg);
        r.fill_text(text_of(pre.to_string(), w, m, m.font, true), at, self.palette.fg, clip);
        fill(r, Rectangle { y: rect.y + rect.height - 2.0, height: 1.5, ..rect }, self.palette.fg);
    }
}

impl<'a, Message: 'a, Theme, R> From<TerminalView<'a, Message>> for Element<'a, Message, Theme, R>
where
    R: text::Renderer<Font = Font> + 'a,
{
    fn from(v: TerminalView<'a, Message>) -> Self {
        Element::new(v)
    }
}
