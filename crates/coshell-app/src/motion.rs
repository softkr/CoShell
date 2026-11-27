//! Small motions for the side panel: cards that pop in one after another, a card that lifts
//! under the pointer, and a command that flies from its snippet card into the dock.

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer;
use iced::advanced::widget::{self, Tree, Widget};
use iced::advanced::{Clipboard, Shell};
use iced::widget::canvas;
use iced::{Color, Element, Event, Length, Point, Rectangle, Renderer, Size, Theme, Transformation, Vector, mouse};

use crate::theme;

/// The cards' entrance: each starts this long after the one above it, and takes `POP` (s).
pub const STAGGER: f32 = 0.045;
pub const POP: f32 = 0.38;
/// A folder's rows: each starts this long after the one above (up to `SLIDE_ROWS` rows,
/// the rest come with the last), takes `SLIDE` (s), from `SLIDE_PX` to one side.
pub const SLIDE_STEP: f32 = 0.018;
pub const SLIDE: f32 = 0.26;
pub const SLIDE_ROWS: usize = 14;
pub const SLIDE_PX: f32 = 28.0;

/// How far row `index` still has to go, `elapsed` seconds after its folder opened: 1 → 0.
pub fn slide(elapsed: f32, index: usize) -> f32 {
    let t = ((elapsed - SLIDE_STEP * index.min(SLIDE_ROWS) as f32) / SLIDE).clamp(0.0, 1.0);
    (1.0 - t).powi(3)
}

/// How long a folder's entrance lasts (s).
pub const SLIDE_LEN: f32 = SLIDE_STEP * SLIDE_ROWS as f32 + SLIDE;

/// How long a command takes to fly into the dock (s).
pub const FLIGHT: f32 = 0.62;

/// How far card `index` is into its entrance, `elapsed` seconds after the list showed: 0 → 1.
pub fn stagger(elapsed: f32, index: usize) -> f32 {
    ((elapsed - STAGGER * index as f32) / POP).clamp(0.0, 1.0)
}

/// How long the entrance of `cards` cards lasts (s).
pub fn stagger_len(cards: usize) -> f32 {
    STAGGER * cards.saturating_sub(1) as f32 + POP
}

/// Fast, a little past the end, then back: a pop.
fn ease_out_back(t: f32) -> f32 {
    let (c1, c3) = (1.70158, 2.70158);
    1.0 + c3 * (t - 1.0).powi(3) + c1 * (t - 1.0).powi(2)
}

/// A card's size and how far below its place it is, `progress` into its entrance.
pub fn pop(progress: f32) -> (f32, f32) {
    let t = progress.clamp(0.0, 1.0);
    (0.82 + 0.18 * ease_out_back(t), 16.0 * (1.0 - t).powi(3))
}

/// Where the flying command is at `t` (0 → 1): an arc that rises out of the card and comes
/// down into the dock.
pub fn arc(from: Point, to: Point, t: f32) -> Point {
    let t = t.clamp(0.0, 1.0);
    let lift = (to.x - from.x).abs().max(120.0) * 0.35;
    let control = Point::new((from.x + to.x) / 2.0, from.y.min(to.y) - lift);
    let u = 1.0 - t;
    Point::new(u * u * from.x + 2.0 * u * t * control.x + t * t * to.x, u * u * from.y + 2.0 * u * t * control.y + t * t * to.y)
}

/// Gentle at both ends.
fn ease_in_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    if t < 0.5 { 4.0 * t * t * t } else { 1.0 - (-2.0 * t + 2.0).powi(3) / 2.0 }
}

/// Draws its content smaller (around its centre) and moved, or not at all; its place in the
/// layout, and what it does with input, stay as they are.
pub struct Shift<'a, Message> {
    content: Element<'a, Message, Theme, Renderer>,
    scale: f32,
    offset: Vector,
    shown: bool,
}

impl<'a, Message> Shift<'a, Message> {
    pub fn new(content: impl Into<Element<'a, Message, Theme, Renderer>>, scale: f32, offset: Vector, shown: bool) -> Self {
        Self { content: content.into(), scale, offset, shown }
    }
}

impl<Message> Widget<Message, Theme, Renderer> for Shift<'_, Message> {
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

    fn draw(&self, tree: &Tree, renderer: &mut Renderer, theme: &Theme, style: &renderer::Style, layout: Layout<'_>, cursor: mouse::Cursor, viewport: &Rectangle) {
        use iced::advanced::Renderer as _;
        if !self.shown {
            return;
        }
        if self.scale == 1.0 && self.offset == Vector::ZERO {
            return self.content.as_widget().draw(&tree.children[0], renderer, theme, style, layout, cursor, viewport);
        }
        let c = layout.bounds().center();
        let place = Transformation::translate(c.x + self.offset.x, c.y + self.offset.y) * Transformation::scale(self.scale) * Transformation::translate(-c.x, -c.y);
        renderer.with_transformation(place, |r| self.content.as_widget().draw(&tree.children[0], r, theme, style, layout, cursor, viewport));
    }

    fn update(&mut self, tree: &mut Tree, event: &Event, layout: Layout<'_>, cursor: mouse::Cursor, renderer: &Renderer, clipboard: &mut dyn Clipboard, shell: &mut Shell<'_, Message>, viewport: &Rectangle) {
        self.content.as_widget_mut().update(&mut tree.children[0], event, layout, cursor, renderer, clipboard, shell, viewport);
    }
    fn mouse_interaction(&self, tree: &Tree, layout: Layout<'_>, cursor: mouse::Cursor, viewport: &Rectangle, renderer: &Renderer) -> mouse::Interaction {
        self.content.as_widget().mouse_interaction(&tree.children[0], layout, cursor, viewport, renderer)
    }
    fn operate(&mut self, tree: &mut Tree, layout: Layout<'_>, renderer: &Renderer, operation: &mut dyn widget::Operation) {
        self.content.as_widget_mut().operate(&mut tree.children[0], layout, renderer, operation);
    }
    fn overlay<'b>(&'b mut self, tree: &'b mut Tree, layout: Layout<'b>, renderer: &Renderer, viewport: &Rectangle, translation: Vector) -> Option<iced::advanced::overlay::Element<'b, Message, Theme, Renderer>> {
        self.content.as_widget_mut().overlay(&mut tree.children[0], layout, renderer, viewport, translation)
    }
}

impl<'a, Message: 'a> From<Shift<'a, Message>> for Element<'a, Message, Theme, Renderer> {
    fn from(s: Shift<'a, Message>) -> Self {
        Element::new(s)
    }
}

/// Seconds since the app started: the clock of motions that never stop (a connected server's
/// pulse). Pure in tests: always the same moment.
pub fn seconds() -> f32 {
    #[cfg(test)]
    return 0.0;
    #[cfg(not(test))]
    {
        static START: std::sync::LazyLock<std::time::Instant> = std::sync::LazyLock::new(std::time::Instant::now);
        START.elapsed().as_secs_f32()
    }
}

/// How long a group takes to fold open or shut (s).
#[cfg_attr(test, allow(dead_code))]
pub const FOLD: f32 = 0.24;

/// Shows `open` (0 → 1) of its content's height, cut off below: a group folding open or shut.
pub struct Fold<'a, Message> {
    content: Element<'a, Message, Theme, Renderer>,
    open: f32,
}

impl<'a, Message> Fold<'a, Message> {
    pub fn new(content: impl Into<Element<'a, Message, Theme, Renderer>>, open: f32) -> Self {
        Self { content: content.into(), open: open.clamp(0.0, 1.0) }
    }
}

impl<Message> Widget<Message, Theme, Renderer> for Fold<'_, Message> {
    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }
    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_ref(&self.content));
    }
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Shrink)
    }
    fn layout(&mut self, tree: &mut Tree, renderer: &Renderer, limits: &layout::Limits) -> layout::Node {
        let content = self.content.as_widget_mut().layout(&mut tree.children[0], renderer, &limits.height(Length::Shrink));
        let size = content.size();
        layout::Node::with_children(Size::new(size.width, size.height * self.open), vec![content])
    }
    fn draw(&self, tree: &Tree, renderer: &mut Renderer, theme: &Theme, style: &renderer::Style, layout: Layout<'_>, cursor: mouse::Cursor, viewport: &Rectangle) {
        use iced::advanced::Renderer as _;
        let Some(visible) = layout.bounds().intersection(viewport) else { return };
        let content = layout.children().next().expect("the content");
        renderer.with_layer(visible, |r| self.content.as_widget().draw(&tree.children[0], r, theme, style, content, cursor, &visible));
    }
    fn update(&mut self, tree: &mut Tree, event: &Event, layout: Layout<'_>, cursor: mouse::Cursor, renderer: &Renderer, clipboard: &mut dyn Clipboard, shell: &mut Shell<'_, Message>, viewport: &Rectangle) {
        // Half folded, nothing in it is offered: it is on its way in or out.
        if self.open >= 1.0 {
            let content = layout.children().next().expect("the content");
            self.content.as_widget_mut().update(&mut tree.children[0], event, content, cursor, renderer, clipboard, shell, viewport);
        }
    }
    fn mouse_interaction(&self, tree: &Tree, layout: Layout<'_>, cursor: mouse::Cursor, viewport: &Rectangle, renderer: &Renderer) -> mouse::Interaction {
        let content = layout.children().next().expect("the content");
        self.content.as_widget().mouse_interaction(&tree.children[0], content, cursor, viewport, renderer)
    }
    fn operate(&mut self, tree: &mut Tree, layout: Layout<'_>, renderer: &Renderer, operation: &mut dyn widget::Operation) {
        let content = layout.children().next().expect("the content");
        self.content.as_widget_mut().operate(&mut tree.children[0], content, renderer, operation);
    }
}

impl<'a, Message: 'a> From<Fold<'a, Message>> for Element<'a, Message, Theme, Renderer> {
    fn from(f: Fold<'a, Message>) -> Self {
        Element::new(f)
    }
}

/// A command on its way from a snippet card (`from`) into the dock (`to`), `progress` along.
pub struct Flight {
    pub from: Point,
    pub to: Point,
    pub cmd: String,
    pub progress: f32,
}

/// The trail's colours, front to back.
const TRAIL: [Color; 3] = [theme::TEAL, theme::BLUE, theme::PURPLE];
/// How many sparks the trail has, and how far behind (in progress) the last one is.
const SPARKS: usize = 14;
const TRAIL_LEN: f32 = 0.22;
/// The flight ends with a ring spreading from where it lands: from this point of it.
const LANDING: f32 = 0.78;

impl<Message> canvas::Program<Message> for Flight {
    type State = ();

    fn draw(&self, _: &(), renderer: &Renderer, _: &Theme, bounds: Rectangle, _: mouse::Cursor) -> Vec<canvas::Geometry> {
        use canvas::{Frame, Path, Stroke, Text};
        let mut frame = Frame::new(renderer, bounds.size());
        let at = |p: Point| Point::new(p.x - bounds.x, p.y - bounds.y);
        let flying = (self.progress / LANDING).min(1.0);
        let head = ease_in_out(flying);
        // The trail: sparks along the way just flown, smaller and fainter the further back.
        if flying < 1.0 {
            for i in (1..=SPARKS).rev() {
                let back = i as f32 / SPARKS as f32;
                let t = head - back * TRAIL_LEN;
                if t <= 0.0 {
                    continue;
                }
                let colour = TRAIL[(i * TRAIL.len() / (SPARKS + 1)).min(TRAIL.len() - 1)];
                let wobble = Vector::new((i as f32 * 2.1 + self.progress * 30.0).sin() * 3.0, (i as f32 * 1.7).cos() * 3.0);
                let p = at(arc(self.from, self.to, t)) + wobble;
                frame.fill(&Path::circle(p, 4.0 * (1.0 - back) + 1.0), Color { a: 0.9 * (1.0 - back), ..colour });
            }
            // The command itself, in a lit pill, shrinking as it nears the dock.
            let p = at(arc(self.from, self.to, head));
            let label: String = format!("$ {}", self.cmd).chars().take(28).collect();
            let size = 12.0 * (1.0 - 0.35 * head);
            let w = label.chars().count() as f32 * size * 0.62 + 18.0;
            let h = size + 12.0;
            let pill = Path::rounded_rectangle(Point::new(p.x - w / 2.0, p.y - h / 2.0), Size::new(w, h), (h / 2.0).into());
            frame.fill(&pill, Color { a: 0.96, ..theme::BG_3 });
            let edge = canvas::Gradient::Linear(canvas::gradient::Linear::new(Point::new(p.x - w / 2.0, p.y), Point::new(p.x + w / 2.0, p.y)).add_stop(0.0, theme::PURPLE).add_stop(0.5, theme::BLUE).add_stop(1.0, theme::TEAL));
            frame.stroke(&pill, Stroke::default().with_width(1.5).with_color(theme::BLUE));
            frame.stroke(&pill, Stroke { style: canvas::Style::Gradient(edge), width: 1.5, ..Stroke::default() });
            frame.fill_text(Text {
                content: label,
                position: p,
                color: theme::ACCENT_TX,
                size: size.into(),
                font: crate::fonts::MONO,
                align_x: iced::alignment::Horizontal::Center.into(),
                align_y: iced::alignment::Vertical::Center,
                ..Text::default()
            });
        }
        // Landed: a ring spreads from the dock and fades.
        if self.progress > LANDING {
            let t = (self.progress - LANDING) / (1.0 - LANDING);
            let ring = Path::circle(at(self.to), 8.0 + 46.0 * t);
            frame.stroke(&ring, Stroke::default().with_width(2.5 * (1.0 - t) + 0.5).with_color(Color { a: 0.8 * (1.0 - t), ..theme::TEAL }));
            frame.fill(&Path::circle(at(self.to), 6.0 * (1.0 - t)), Color { a: 0.9 * (1.0 - t), ..theme::BLUE });
        }
        vec![frame.into_geometry()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cards_pop_in_one_after_another() {
        assert_eq!(stagger(0.0, 0), 0.0);
        assert_eq!(stagger(STAGGER, 1), 0.0, "the second waits for its turn");
        assert!(stagger(0.1, 0) > stagger(0.1, 1));
        assert!(stagger(stagger_len(7), 6) > 0.9999, "the last is in place when the entrance is over");
        let (scale, below) = pop(0.0);
        assert!(scale < 0.9 && below > 10.0);
        assert_eq!(pop(1.0), (1.0, 0.0));
        assert!((0..=20).map(|i| pop(i as f32 / 20.0).0).any(|s| s > 1.0), "it overshoots a little: a pop");
    }

    #[test]
    fn rows_slide_in_one_after_another() {
        assert_eq!(slide(0.0, 0), 1.0, "all the way out at first");
        assert!(slide(0.1, 0) < slide(0.1, 3), "the first row is ahead");
        assert_eq!(slide(SLIDE_LEN, 100), 0.0, "even a long folder is in place when it is over");
        assert_eq!(slide(SLIDE_LEN, 100), slide(SLIDE_LEN, SLIDE_ROWS), "rows past the first few come together");
    }

    #[test]
    fn the_command_arcs_from_the_card_to_the_dock() {
        let (from, to) = (Point::new(200.0, 300.0), Point::new(700.0, 800.0));
        assert_eq!(arc(from, to, 0.0), from);
        assert_eq!(arc(from, to, 1.0), to);
        assert!(arc(from, to, 0.2).y < from.y, "it rises out of the card first");
        let xs: Vec<f32> = (0..=10).map(|i| arc(from, to, i as f32 / 10.0).x).collect();
        assert!(xs.windows(2).all(|w| w[1] >= w[0]), "always toward the dock");
    }
}
