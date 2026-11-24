//! Read-only styled text that can be selected with the mouse and copied with ⌘C
//! (Ctrl+Shift+C elsewhere, as in the terminal). Drag selects characters, a double
//! click a word, a triple click a line, and Shift+click extends the selection.
//!
//! It never takes keyboard focus: keys other than copy keep going to the shell.
//! Each line is its own paragraph, so a hit can be told apart by line and byte.

use std::borrow::Cow;

use iced::advanced::layout::{self, Layout};
use iced::advanced::text::{self, Paragraph, Span};
use iced::advanced::widget::{Operation, Tree, Widget, tree};
use iced::advanced::{Clipboard, Shell, mouse, renderer};
use iced::keyboard::{self, Modifiers};
use iced::widget::text::{Alignment, LineHeight, Shaping, Wrapping};
use iced::{Background, Border, Color, Element, Event, Font, Length, Pixels, Point, Rectangle, Size, Vector, alignment};

/// A place in the text: line, and byte offset within it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Pos {
    pub line: usize,
    pub byte: usize,
}

impl Pos {
    pub const fn new(line: usize, byte: usize) -> Self {
        Self { line, byte }
    }
}

/// The bytes of `line` (of length `len`) that the selection `a..b` covers.
pub fn range_in_line(line: usize, len: usize, a: Pos, b: Pos) -> Option<(usize, usize)> {
    let (a, b) = if a <= b { (a, b) } else { (b, a) };
    if line < a.line || line > b.line {
        return None;
    }
    let start = if line == a.line { a.byte.min(len) } else { 0 };
    let end = if line == b.line { b.byte.min(len) } else { len };
    (start < end).then_some((start, end))
}

/// The text between two positions, lines joined by `\n`.
pub fn selected_text(lines: &[&str], a: Pos, b: Pos) -> String {
    let (a, b) = if a <= b { (a, b) } else { (b, a) };
    let mut out = String::new();
    for (i, line) in lines.iter().enumerate().take(b.line + 1).skip(a.line) {
        if i > a.line {
            out.push('\n');
        }
        if let Some((s, e)) = range_in_line(i, line.len(), a, b) {
            out.push_str(&line[s..e]);
        }
    }
    out
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || "_-./~:@%+=#".contains(c)
}

/// The word around `byte`: a run of word characters, or the single character there.
pub fn word_at(line: &str, byte: usize) -> (usize, usize) {
    let byte = floor_boundary(line, byte);
    let Some(c) = line[byte..].chars().next() else { return (byte, byte) };
    if !is_word(c) {
        return (byte, byte + c.len_utf8());
    }
    let start = line[..byte].char_indices().rev().take_while(|&(_, c)| is_word(c)).last().map_or(byte, |(i, _)| i);
    let end = line[byte..].char_indices().find(|&(_, c)| !is_word(c)).map_or(line.len(), |(i, _)| byte + i);
    (start, end)
}

fn floor_boundary(s: &str, i: usize) -> usize {
    let mut i = i.min(s.len());
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

type StaticSpan = Span<'static, (), Font>;

/// `spans` cut at the selection's edges, the selected part on a highlight.
fn with_selection(spans: &[StaticSpan], sel: Option<(usize, usize)>, color: Color) -> Vec<StaticSpan> {
    let Some((a, b)) = sel else { return spans.to_vec() };
    let mut out = Vec::with_capacity(spans.len() + 2);
    let mut at = 0;
    for s in spans {
        let t: &str = &s.text;
        let (s0, s1) = (at, at + t.len());
        at = s1;
        let cuts = [s0, a.clamp(s0, s1), b.clamp(s0, s1), s1];
        for w in cuts.windows(2) {
            if w[0] >= w[1] {
                continue;
            }
            let mut part = s.clone();
            part.text = Cow::Owned(t[w[0] - s0..w[1] - s0].to_string());
            if w[0] >= a && w[1] <= b {
                part.highlight = Some(text::Highlight { background: Background::Color(color), border: Border::default() });
            }
            out.push(part);
        }
    }
    out
}

pub struct Selectable<'a, Message> {
    lines: Vec<Vec<Span<'a, (), Font>>>,
    font: Font,
    size: Pixels,
    line_height: f32,
    selection: Color,
    on_copy: Box<dyn Fn(String) -> Message + 'a>,
}

impl<'a, Message> Selectable<'a, Message> {
    pub fn new(lines: Vec<Vec<Span<'a, (), Font>>>, on_copy: impl Fn(String) -> Message + 'a) -> Self {
        Self {
            lines,
            font: Font::default(),
            size: Pixels(13.0),
            line_height: 20.0,
            selection: Color::from_rgba(0.3, 0.5, 1.0, 0.35),
            on_copy: Box::new(on_copy),
        }
    }

    pub fn font(mut self, font: Font) -> Self {
        self.font = font;
        self
    }

    pub fn size(mut self, size: f32) -> Self {
        self.size = Pixels(size);
        self
    }

    pub fn line_height(mut self, px: f32) -> Self {
        self.line_height = px;
        self
    }

    pub fn selection(mut self, color: Color) -> Self {
        self.selection = color;
        self
    }
}

struct Line<P> {
    source: Vec<StaticSpan>,
    text: String,
    /// What the paragraph was built with.
    sel: Option<(usize, usize)>,
    drawn: Vec<StaticSpan>,
    paragraph: P,
    y: f32,
    h: f32,
}

struct State<P> {
    lines: Vec<Line<P>>,
    width: f32,
    anchor: Option<Pos>,
    head: Option<Pos>,
    dragging: bool,
    last_click: Option<mouse::Click>,
    mods: Modifiers,
}

impl<P: Paragraph<Font = Font>> State<P> {
    fn selection(&self) -> Option<(Pos, Pos)> {
        let (a, b) = (self.anchor?, self.head?);
        (a != b).then_some((a, b))
    }

    fn text(&self) -> Option<String> {
        let (a, b) = self.selection()?;
        let lines: Vec<&str> = self.lines.iter().map(|l| l.text.as_str()).collect();
        Some(selected_text(&lines, a, b)).filter(|t| !t.is_empty())
    }

    /// Where `p` (relative to the widget) falls; points outside clamp to the ends.
    fn hit(&self, p: Point) -> Pos {
        let Some(last) = self.lines.len().checked_sub(1) else { return Pos::new(0, 0) };
        if p.y < 0.0 {
            return Pos::new(0, 0);
        }
        let Some(i) = self.lines.iter().position(|l| p.y < l.y + l.h) else {
            return Pos::new(last, self.lines[last].text.len());
        };
        let line = &self.lines[i];
        if p.x <= 0.0 {
            return Pos::new(i, 0);
        }
        let local = Point::new(p.x, (p.y - line.y).max(0.0));
        let byte = line.paragraph.hit_test(local).map_or(line.text.len(), |h| h.cursor());
        Pos::new(i, floor_boundary(&line.text, byte))
    }

    fn text_spec<'s>(&self, spans: &'s [StaticSpan], w: &Selectable<'_, impl Sized>) -> text::Text<&'s [StaticSpan], Font> {
        text::Text {
            content: spans,
            bounds: Size::new(self.width, f32::INFINITY),
            size: w.size,
            line_height: LineHeight::Absolute(w.line_height.into()),
            font: w.font,
            align_x: Alignment::Default,
            align_y: alignment::Vertical::Top,
            shaping: Shaping::Advanced,
            wrapping: Wrapping::Word,
        }
    }

    /// Rebuilds the lines whose selected part changed (same text, so same layout).
    fn refresh(&mut self, w: &Selectable<'_, impl Sized>) {
        let sel = self.selection();
        for i in 0..self.lines.len() {
            let line = &self.lines[i];
            let want = sel.and_then(|(a, b)| range_in_line(i, line.text.len(), a, b));
            if want != line.sel {
                let drawn = with_selection(&line.source, want, w.selection);
                let paragraph = P::with_spans(self.text_spec(&drawn, w));
                let line = &mut self.lines[i];
                (line.sel, line.drawn, line.paragraph) = (want, drawn, paragraph);
            }
        }
    }

    fn clear(&mut self) -> bool {
        let had = self.selection().is_some();
        (self.anchor, self.head, self.dragging) = (None, None, false);
        had
    }
}

fn is_copy(key: &keyboard::Key, physical: keyboard::key::Physical, mods: Modifiers) -> bool {
    mods.command() && key.to_latin(physical) == Some('c') && (cfg!(target_os = "macos") || mods.shift())
}

impl<Message, Theme, R> Widget<Message, Theme, R> for Selectable<'_, Message>
where
    R: text::Renderer<Font = Font>,
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State<R::Paragraph>>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::<R::Paragraph> {
            lines: Vec::new(),
            width: 0.0,
            anchor: None,
            head: None,
            dragging: false,
            last_click: None,
            mods: Modifiers::default(),
        })
    }

    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Shrink)
    }

    fn layout(&mut self, tree: &mut Tree, _renderer: &R, limits: &layout::Limits) -> layout::Node {
        let state = tree.state.downcast_mut::<State<R::Paragraph>>();
        let width = limits.max().width;
        let resized = width != state.width;
        state.width = width;
        state.lines.truncate(self.lines.len());
        let sel = state.selection();
        let mut y = 0.0;
        for (i, spans) in self.lines.iter().enumerate() {
            let same = state.lines.get(i).is_some_and(|l| l.source.as_slice() == spans.as_slice());
            if !same {
                let source: Vec<StaticSpan> = spans.iter().cloned().map(Span::to_static).collect();
                let text: String = source.iter().map(|s| s.text.as_ref()).collect();
                let want = sel.and_then(|(a, b)| range_in_line(i, text.len(), a, b));
                let drawn = with_selection(&source, want, self.selection);
                let paragraph = R::Paragraph::with_spans(state.text_spec(&drawn, self));
                let line = Line { source, text, sel: want, drawn, paragraph, y: 0.0, h: 0.0 };
                if i < state.lines.len() {
                    state.lines[i] = line;
                } else {
                    state.lines.push(line);
                }
            } else if resized {
                state.lines[i].paragraph.resize(Size::new(width, f32::INFINITY));
            }
            let line = &mut state.lines[i];
            line.y = y;
            line.h = line.paragraph.min_bounds().height.max(self.line_height);
            y += line.h;
        }
        state.refresh(self);
        layout::Node::new(Size::new(width, y))
    }

    fn draw(&self, tree: &Tree, renderer: &mut R, _theme: &Theme, style: &renderer::Style, layout: Layout<'_>, _cursor: mouse::Cursor, viewport: &Rectangle) {
        let bounds = layout.bounds();
        if !bounds.intersects(viewport) {
            return;
        }
        let state = tree.state.downcast_ref::<State<R::Paragraph>>();
        for line in &state.lines {
            let origin = Point::new(bounds.x, bounds.y + line.y);
            if !Rectangle::new(origin, Size::new(bounds.width, line.h)).intersects(viewport) {
                continue;
            }
            let shift = Vector::new(origin.x, origin.y);
            for (i, span) in line.drawn.iter().enumerate() {
                if let Some(h) = span.highlight {
                    for r in line.paragraph.span_bounds(i) {
                        renderer.fill_quad(renderer::Quad { bounds: r + shift, border: h.border, ..Default::default() }, h.background);
                    }
                }
            }
            renderer.fill_paragraph(&line.paragraph, origin, style.text_color, *viewport);
        }
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _renderer: &R,
        _clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        _viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_mut::<State<R::Paragraph>>();
        let bounds = layout.bounds();
        let origin = Vector::new(bounds.x, bounds.y);
        match event {
            Event::Keyboard(keyboard::Event::ModifiersChanged(m)) => state.mods = *m,
            // The press is not captured: the click also gives the keyboard back to the shell.
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => match cursor.position_over(bounds) {
                Some(p) => {
                    let pos = state.hit(p - origin);
                    let click = mouse::Click::new(p, mouse::Button::Left, state.last_click);
                    state.last_click = Some(click);
                    match click.kind() {
                        mouse::click::Kind::Single if state.mods.shift() && state.anchor.is_some() => state.head = Some(pos),
                        mouse::click::Kind::Single => (state.anchor, state.head) = (Some(pos), Some(pos)),
                        mouse::click::Kind::Double => {
                            let (s, e) = state.lines.get(pos.line).map_or((0, 0), |l| word_at(&l.text, pos.byte));
                            (state.anchor, state.head) = (Some(Pos::new(pos.line, s)), Some(Pos::new(pos.line, e)));
                        }
                        mouse::click::Kind::Triple => {
                            let len = state.lines.get(pos.line).map_or(0, |l| l.text.len());
                            (state.anchor, state.head) = (Some(Pos::new(pos.line, 0)), Some(Pos::new(pos.line, len)));
                        }
                    }
                    state.dragging = true;
                    state.refresh(self);
                    shell.request_redraw();
                }
                None => {
                    if state.clear() {
                        state.refresh(self);
                        shell.request_redraw();
                    }
                }
            },
            Event::Mouse(mouse::Event::CursorMoved { .. }) if state.dragging => {
                // Past the edges (or outside the scroll area) still extends the selection.
                if let Some(p) = cursor.land().position() {
                    let pos = state.hit(p - origin);
                    if state.head != Some(pos) {
                        state.head = Some(pos);
                        state.refresh(self);
                        shell.request_redraw();
                    }
                }
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) if state.dragging => {
                state.dragging = false;
                if state.selection().is_none() {
                    state.clear();
                }
            }
            Event::Keyboard(keyboard::Event::KeyPressed { key, physical_key, modifiers, .. }) if is_copy(key, *physical_key, *modifiers) => {
                if let Some(t) = state.text() {
                    shell.publish((self.on_copy)(t));
                    shell.capture_event();
                }
            }
            _ => {}
        }
    }

    fn mouse_interaction(&self, _tree: &Tree, layout: Layout<'_>, cursor: mouse::Cursor, _viewport: &Rectangle, _renderer: &R) -> mouse::Interaction {
        if cursor.is_over(layout.bounds()) { mouse::Interaction::Text } else { mouse::Interaction::None }
    }

    fn operate(&mut self, tree: &mut Tree, layout: Layout<'_>, _renderer: &R, operation: &mut dyn Operation) {
        let state = tree.state.downcast_ref::<State<R::Paragraph>>();
        let b = layout.bounds();
        for line in &state.lines {
            let w = line.paragraph.min_bounds().width;
            operation.text(None, Rectangle::new(Point::new(b.x, b.y + line.y), Size::new(w, line.h)), &line.text);
        }
    }
}

impl<'a, Message: 'a, Theme: 'a, R> From<Selectable<'a, Message>> for Element<'a, Message, Theme, R>
where
    R: text::Renderer<Font = Font> + 'a,
{
    fn from(w: Selectable<'a, Message>) -> Self {
        Element::new(w)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_covers_whole_middle_lines_and_parts_of_the_ends() {
        let lines = ["Reading package lists", "", "All up to date."];
        let (a, b) = (Pos::new(0, 8), Pos::new(2, 3));
        assert_eq!(selected_text(&lines, a, b), "package lists\n\nAll");
        assert_eq!(selected_text(&lines, b, a), "package lists\n\nAll", "drag backwards");
        assert_eq!(range_in_line(1, 0, a, b), None);
        assert_eq!(range_in_line(0, 21, a, b), Some((8, 21)));
        assert_eq!(range_in_line(2, 15, a, b), Some((0, 3)));
        assert_eq!(selected_text(&lines, Pos::new(0, 3), Pos::new(0, 3)), "");
    }

    #[test]
    fn words_include_paths_and_korean() {
        let s = "rm /var/log/syslog.1 && 디스크 정리";
        let at = |w: &str| s.find(w).unwrap() + 1;
        let word = |b: usize| {
            let (x, y) = word_at(s, b);
            &s[x..y]
        };
        assert_eq!(word(at("var")), "/var/log/syslog.1");
        assert_eq!(word(at("디스크")), "디스크");
        assert_eq!(word(s.find(' ').unwrap()), " ");
        assert_eq!(word(s.len()), "");
    }

    #[test]
    fn selection_splits_spans_without_changing_the_text() {
        let spans: Vec<StaticSpan> = vec![Span::new("abc"), Span::new("defg")];
        let out = with_selection(&spans, Some((2, 5)), Color::WHITE);
        let parts: Vec<(&str, bool)> = out.iter().map(|s| (s.text.as_ref(), s.highlight.is_some())).collect();
        assert_eq!(parts, [("ab", false), ("c", true), ("de", true), ("fg", false)]);
    }
}
