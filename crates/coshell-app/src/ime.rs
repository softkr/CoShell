//! On-the-spot IME for every text field.
//!
//! iced shows the text being composed (e.g. Korean `ㄴ` before it becomes `나`)
//! in a boxed, underlined overlay. This wrapper sits at the root of the view,
//! takes the composing text out of the input-method request (so the runtime
//! draws nothing) and draws it itself as plain text at the cursor, the way
//! native macOS fields show it.
//!
//! It also commits the composing text when the caret leaves it (click, arrow
//! keys, Tab, Enter, Esc, window losing focus): iced would otherwise drop the
//! last, still-composing syllable.

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer;
use iced::advanced::text::{self, Paragraph as _, Renderer as _};
use iced::advanced::widget::{self, Tree, Widget};
use iced::advanced::{Clipboard, InputMethod, Shell, input_method, mouse};
use iced::keyboard::{self, key::Named};
use iced::{Color, Element, Event, Font, Length, Pixels, Point, Rectangle, Size, Theme, window};

type Renderer = iced::Renderer;
type Paragraph = <Renderer as text::Renderer>::Paragraph;

/// Width of the caret drawn after the composing text.
const CARET_W: f32 = 1.0;
/// A commit from the system this soon after we committed the same text ourselves is the same syllable.
const DUPLICATE_WINDOW: std::time::Duration = std::time::Duration::from_millis(500);

pub struct Ime<'a, Message> {
    content: Element<'a, Message, Theme, Renderer>,
    /// Text and caret colour.
    color: Color,
    /// Background of the text fields: hides the field's own caret, which stays in
    /// front of the composing text, so the caret can be drawn after it.
    field: Color,
    on_compose: Option<Box<dyn Fn(bool) -> Message + 'a>>,
}

impl<'a, Message> Ime<'a, Message> {
    pub fn new(content: impl Into<Element<'a, Message, Theme, Renderer>>, color: Color, field: Color) -> Self {
        Self { content: content.into(), color, field, on_compose: None }
    }

    /// Told when composing starts and ends (multi-line editors hide their placeholder meanwhile).
    pub fn on_compose(mut self, f: impl Fn(bool) -> Message + 'a) -> Self {
        self.on_compose = Some(Box::new(f));
        self
    }
}

#[derive(Default)]
struct State {
    /// (text being composed, where the cursor is, text size)
    composing: Option<(String, Rectangle, Option<Pixels>)>,
    /// What we committed on the user's behalf, and when.
    committed: Option<(String, std::time::Instant)>,
}

/// Moves the caret or the focus away from the composing text.
fn ends_composition(event: &Event) -> bool {
    match event {
        Event::Mouse(mouse::Event::ButtonPressed(_)) | Event::Window(window::Event::Unfocused) => true,
        Event::Keyboard(keyboard::Event::KeyPressed { key: keyboard::Key::Named(k), .. }) => matches!(
            k,
            Named::ArrowLeft
                | Named::ArrowRight
                | Named::ArrowUp
                | Named::ArrowDown
                | Named::Home
                | Named::End
                | Named::PageUp
                | Named::PageDown
                | Named::Tab
                | Named::Enter
                | Named::Escape
        ),
        _ => false,
    }
}

impl<Message> Widget<Message, Theme, Renderer> for Ime<'_, Message> {
    fn tag(&self) -> widget::tree::Tag {
        widget::tree::Tag::of::<State>()
    }
    fn state(&self) -> widget::tree::State {
        widget::tree::State::new(State::default())
    }
    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }
    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_ref(&self.content));
    }
    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }
    fn layout(&mut self, tree: &mut Tree, renderer: &Renderer, limits: &layout::Limits) -> layout::Node {
        self.content.as_widget_mut().layout(&mut tree.children[0], renderer, limits)
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_mut::<State>();
        let was = state.composing.is_some();
        if let Event::InputMethod(input_method::Event::Commit(text)) = event
            && let Some((done, at)) = state.committed.take()
            && done == *text
            && at.elapsed() < DUPLICATE_WINDOW
        {
            shell.capture_event(); // the system committing what we already inserted
            return;
        }
        if ends_composition(event)
            && let Some((text, ..)) = state.composing.take()
        {
            // Into the focused field first, then let the click / key move the caret. A shell of its
            // own: the field marks the commit as handled, which must not swallow the click itself.
            let mut messages = Vec::new();
            {
                let mut side = Shell::new(&mut messages);
                for e in [input_method::Event::Commit(text.clone()), input_method::Event::Preedit(String::new(), None)] {
                    self.content.as_widget_mut().update(&mut tree.children[0], &Event::InputMethod(e), layout, cursor, renderer, clipboard, &mut side, viewport);
                }
            }
            for m in messages {
                shell.publish(m);
            }
            shell.request_redraw();
            tree.state.downcast_mut::<State>().committed = Some((text, std::time::Instant::now()));
        }
        self.content.as_widget_mut().update(&mut tree.children[0], event, layout, cursor, renderer, clipboard, shell, viewport);
        let state = tree.state.downcast_mut::<State>();
        match shell.input_method_mut() {
            InputMethod::Enabled { cursor, preedit, .. } => {
                // Keep it for drawing; the runtime overlay is not wanted.
                state.composing = preedit.take().filter(|p| !p.content.is_empty()).map(|p| (p.content, *cursor, p.text_size));
            }
            InputMethod::Disabled if matches!(event, Event::Window(window::Event::RedrawRequested(_))) => state.composing = None,
            InputMethod::Disabled => {}
        }
        let now = state.composing.is_some();
        if now != was
            && let Some(f) = &self.on_compose
        {
            shell.publish(f(now));
        }
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.content.as_widget().draw(&tree.children[0], renderer, theme, style, layout, cursor, viewport);
        let Some((content, at, size)) = &tree.state.downcast_ref::<State>().composing else { return };
        let size = size.unwrap_or_else(|| renderer.default_size());
        let line = f32::from(size) * 1.3;
        let top = at.y + (at.height - line) / 2.0;
        let room = Rectangle::new(Point::new(at.x, top), Size::new(viewport.x + viewport.width - at.x, line));
        // Measured only (drawing uses owned text: a paragraph built here would be gone before the frame renders).
        let width = Paragraph::with_text(text::Text {
            content: content.as_str(),
            bounds: Size::INFINITE,
            size,
            line_height: text::LineHeight::Absolute(Pixels(line)),
            font: Font::DEFAULT,
            align_x: text::Alignment::Default,
            align_y: iced::alignment::Vertical::Top,
            shaping: text::Shaping::Advanced,
            wrapping: text::Wrapping::None,
        })
        .min_width();
        renderer::Renderer::with_layer(renderer, *viewport, |renderer| {
            let quad = |bounds| renderer::Quad { bounds, ..Default::default() };
            // The field's caret (in front of the composing text) and what lies under it.
            renderer::Renderer::fill_quad(renderer, quad(Rectangle::new(Point::new(at.x - 1.0, at.y), Size::new(width + CARET_W + 2.0, at.height))), self.field);
            renderer::Renderer::fill_quad(renderer, quad(Rectangle::new(Point::new(at.x + width, at.y), Size::new(CARET_W, at.height))), self.color);
            renderer.fill_text(
                text::Text {
                    content: content.clone(),
                    bounds: room.size(),
                    size,
                    line_height: text::LineHeight::Absolute(Pixels(line)),
                    font: Font::DEFAULT,
                    align_x: text::Alignment::Default,
                    align_y: iced::alignment::Vertical::Top,
                    shaping: text::Shaping::Advanced,
                    wrapping: text::Wrapping::None,
                },
                room.position(),
                self.color,
                room,
            );
        });
    }

    fn mouse_interaction(&self, tree: &Tree, layout: Layout<'_>, cursor: mouse::Cursor, viewport: &Rectangle, renderer: &Renderer) -> mouse::Interaction {
        self.content.as_widget().mouse_interaction(&tree.children[0], layout, cursor, viewport, renderer)
    }

    fn operate(&mut self, tree: &mut Tree, layout: Layout<'_>, renderer: &Renderer, operation: &mut dyn widget::Operation) {
        self.content.as_widget_mut().operate(&mut tree.children[0], layout, renderer, operation);
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: iced::Vector,
    ) -> Option<iced::advanced::overlay::Element<'b, Message, Theme, Renderer>> {
        self.content.as_widget_mut().overlay(&mut tree.children[0], layout, renderer, viewport, translation)
    }
}

impl<'a, Message: 'a> From<Ime<'a, Message>> for Element<'a, Message, Theme, Renderer> {
    fn from(w: Ime<'a, Message>) -> Self {
        Element::new(w)
    }
}
