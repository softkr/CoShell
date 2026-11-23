//! Right-click menu: wraps an element, reports a secondary click on it and,
//! while the app passes a menu, floats that menu where the click landed.
//!
//! The click point is kept in the widget's own state (relative to the element),
//! so the app only tracks *which* menu is open. Drawn as an overlay: it stays
//! above everything and follows the element when its list scrolls.

use iced::advanced::layout::{self, Layout};
use iced::advanced::widget::{self, Tree, Widget};
use iced::advanced::{Clipboard, Shell, mouse, overlay, renderer};
use iced::{Element, Event, Length, Point, Rectangle, Size, Theme, Vector, window};

type Renderer = iced::Renderer;

pub struct ContextMenu<'a, Message> {
    content: Element<'a, Message, Theme, Renderer>,
    menu: Option<Element<'a, Message, Theme, Renderer>>,
    on_open: Message,
    on_close: Message,
}

impl<'a, Message: Clone> ContextMenu<'a, Message> {
    /// `on_open` is sent on a right click, `on_close` on a click elsewhere, a scroll
    /// or the window losing focus (Esc is the app's: its shortcuts see every key).
    pub fn new(content: impl Into<Element<'a, Message, Theme, Renderer>>, on_open: Message, on_close: Message) -> Self {
        Self { content: content.into(), menu: None, on_open, on_close }
    }

    /// The menu to show (the app passes it only while this one is open).
    pub fn menu(mut self, menu: Option<Element<'a, Message, Theme, Renderer>>) -> Self {
        self.menu = menu;
        self
    }
}

/// Where the right click landed, relative to the element.
#[derive(Default)]
struct State {
    at: Point,
}

impl<Message: Clone> Widget<Message, Theme, Renderer> for ContextMenu<'_, Message> {
    fn tag(&self) -> widget::tree::Tag {
        widget::tree::Tag::of::<State>()
    }
    fn state(&self) -> widget::tree::State {
        widget::tree::State::new(State::default())
    }
    fn children(&self) -> Vec<Tree> {
        std::iter::once(&self.content).chain(&self.menu).map(Tree::new).collect()
    }
    fn diff(&self, tree: &mut Tree) {
        let children: Vec<&Element<'_, Message, Theme, Renderer>> = std::iter::once(&self.content).chain(&self.menu).collect();
        tree.diff_children(&children);
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
        if let Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right)) = event
            && let Some(at) = cursor.position_in(layout.bounds())
        {
            tree.state.downcast_mut::<State>().at = at;
            shell.publish(self.on_open.clone());
        }
        self.content.as_widget_mut().update(&mut tree.children[0], event, layout, cursor, renderer, clipboard, shell, viewport);
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
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        let at = tree.state.downcast_ref::<State>().at;
        let anchor = layout.position() + translation + Vector::new(at.x, at.y);
        let (first, rest) = tree.children.split_at_mut(1);
        let content = self.content.as_widget_mut().overlay(&mut first[0], layout, renderer, viewport, translation);
        let menu = self.menu.as_mut().zip(rest.first_mut()).map(|(menu, tree)| {
            overlay::Element::new(Box::new(Menu { menu, tree, anchor, on_close: self.on_close.clone() }))
        });
        if content.is_none() && menu.is_none() {
            return None;
        }
        Some(overlay::Group::with_children(content.into_iter().chain(menu).collect()).overlay())
    }
}

struct Menu<'a, 'b, Message> {
    menu: &'a mut Element<'b, Message, Theme, Renderer>,
    tree: &'a mut Tree,
    /// The click point, in window coordinates.
    anchor: Point,
    on_close: Message,
}

impl<Message: Clone> overlay::Overlay<Message, Theme, Renderer> for Menu<'_, '_, Message> {
    fn layout(&mut self, renderer: &Renderer, bounds: Size) -> layout::Node {
        let node = self.menu.as_widget_mut().layout(self.tree, renderer, &layout::Limits::new(Size::ZERO, bounds));
        let size = node.size();
        // Kept inside the window: flipped up near the bottom, pushed left near the right edge.
        let x = self.anchor.x.min(bounds.width - size.width).max(0.0);
        let y = if self.anchor.y + size.height > bounds.height { self.anchor.y - size.height } else { self.anchor.y }.max(0.0);
        node.move_to(Point::new(x, y))
    }

    fn update(
        &mut self,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
    ) {
        let outside = !cursor.is_over(layout.bounds());
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(button)) if outside => {
                shell.publish(self.on_close.clone());
                // A left click only dismisses, like a native menu; a right click may open another.
                if *button != mouse::Button::Right {
                    shell.capture_event();
                }
                return;
            }
            Event::Mouse(mouse::Event::WheelScrolled { .. }) | Event::Window(window::Event::Unfocused) => {
                shell.publish(self.on_close.clone());
                shell.capture_event();
                return;
            }
            _ => {}
        }
        self.menu.as_widget_mut().update(self.tree, event, layout, cursor, renderer, clipboard, shell, &layout.bounds());
    }

    fn operate(&mut self, layout: Layout<'_>, renderer: &Renderer, operation: &mut dyn widget::Operation) {
        self.menu.as_widget_mut().operate(self.tree, layout, renderer, operation);
    }

    fn draw(&self, renderer: &mut Renderer, theme: &Theme, style: &renderer::Style, layout: Layout<'_>, cursor: mouse::Cursor) {
        self.menu.as_widget().draw(self.tree, renderer, theme, style, layout, cursor, &layout.bounds());
    }

    fn mouse_interaction(&self, layout: Layout<'_>, cursor: mouse::Cursor, renderer: &Renderer) -> mouse::Interaction {
        self.menu.as_widget().mouse_interaction(self.tree, layout, cursor, &layout.bounds(), renderer)
    }
}

impl<'a, Message: Clone + 'a> From<ContextMenu<'a, Message>> for Element<'a, Message, Theme, Renderer> {
    fn from(w: ContextMenu<'a, Message>) -> Self {
        Element::new(w)
    }
}
