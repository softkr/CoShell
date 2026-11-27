//! A window onto content that is wider than it: the content is laid out at its own
//! full width whatever room the window has, centred, and cut off at the window's
//! edges. Growing the window uncovers the content without re-wrapping it, which is
//! what lets a panel open smoothly.

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer;
use iced::advanced::widget::{self, Tree, Widget};
use iced::advanced::{Clipboard, Shell};
use iced::{Element, Event, Length, Rectangle, Renderer, Size, Theme, Vector, mouse};

pub struct Reveal<'a, Message> {
    content: Element<'a, Message, Theme, Renderer>,
    /// The width the content is laid out at.
    full_width: f32,
    /// The window's own width and its greatest height.
    width: f32,
    max_height: f32,
}

impl<'a, Message> Reveal<'a, Message> {
    pub fn new(content: impl Into<Element<'a, Message, Theme, Renderer>>, full_width: f32, width: f32, max_height: f32) -> Self {
        Self { content: content.into(), full_width, width, max_height }
    }
}

impl<Message> Widget<Message, Theme, Renderer> for Reveal<'_, Message> {
    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }
    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_ref(&self.content));
    }
    fn size(&self) -> Size<Length> {
        Size::new(Length::Shrink, Length::Shrink)
    }

    fn layout(&mut self, tree: &mut Tree, renderer: &Renderer, limits: &layout::Limits) -> layout::Node {
        // The content has all the width it will ever have, and whatever height it wants.
        let inner = layout::Limits::new(Size::ZERO, Size::new(self.full_width, f32::INFINITY));
        let content = self.content.as_widget_mut().layout(&mut tree.children[0], renderer, &inner);
        let width = self.width.min(limits.max().width);
        let height = content.size().height.min(self.max_height).min(limits.max().height);
        // Centred: what does not fit is cut off equally on both sides.
        let content = content.move_to(iced::Point::new((width - self.full_width) / 2.0, 0.0));
        layout::Node::with_children(Size::new(width, height), vec![content])
    }

    fn draw(&self, tree: &Tree, renderer: &mut Renderer, theme: &Theme, style: &renderer::Style, layout: Layout<'_>, cursor: mouse::Cursor, viewport: &Rectangle) {
        use iced::advanced::Renderer as _;
        let bounds = layout.bounds();
        let Some(visible) = bounds.intersection(viewport) else { return };
        let content = layout.children().next().expect("the content");
        renderer.with_layer(visible, |renderer| {
            self.content.as_widget().draw(&tree.children[0], renderer, theme, style, content, cursor, &visible);
        });
    }

    fn update(&mut self, tree: &mut Tree, event: &Event, layout: Layout<'_>, cursor: mouse::Cursor, renderer: &Renderer, clipboard: &mut dyn Clipboard, shell: &mut Shell<'_, Message>, viewport: &Rectangle) {
        let content = layout.children().next().expect("the content");
        self.content.as_widget_mut().update(&mut tree.children[0], event, content, cursor, renderer, clipboard, shell, viewport);
    }

    fn mouse_interaction(&self, tree: &Tree, layout: Layout<'_>, cursor: mouse::Cursor, viewport: &Rectangle, renderer: &Renderer) -> mouse::Interaction {
        let content = layout.children().next().expect("the content");
        self.content.as_widget().mouse_interaction(&tree.children[0], content, cursor, viewport, renderer)
    }

    fn operate(&mut self, tree: &mut Tree, layout: Layout<'_>, renderer: &Renderer, operation: &mut dyn widget::Operation) {
        let content = layout.children().next().expect("the content");
        self.content.as_widget_mut().operate(&mut tree.children[0], content, renderer, operation);
    }

    fn overlay<'b>(&'b mut self, tree: &'b mut Tree, layout: Layout<'b>, renderer: &Renderer, viewport: &Rectangle, translation: Vector) -> Option<iced::advanced::overlay::Element<'b, Message, Theme, Renderer>> {
        let content = layout.children().next().expect("the content");
        self.content.as_widget_mut().overlay(&mut tree.children[0], content, renderer, viewport, translation)
    }
}

impl<'a, Message: 'a> From<Reveal<'a, Message>> for Element<'a, Message, Theme, Renderer> {
    fn from(r: Reveal<'a, Message>) -> Self {
        Element::new(r)
    }
}
