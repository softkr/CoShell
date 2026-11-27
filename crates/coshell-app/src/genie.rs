//! The genie (요술 램프): a panel pours out of a point and is sucked back into it, the way
//! macOS minimises a window into the Dock.
//!
//! iced cannot turn a widget into a picture to bend, so the panel is drawn many times over,
//! each time cut to one thin horizontal strip with its own size and place: stacked, the strips
//! make the funnel. The panel is laid out once, at its final place, so nothing in it re-wraps
//! while it moves.

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer;
use iced::advanced::widget::{self, Tree, Widget};
use iced::advanced::{Clipboard, Shell};
use iced::{Element, Event, Length, Padding, Point, Rectangle, Renderer, Size, Theme, Transformation, Vector, mouse};

/// One slice of the panel and where it is drawn: `src` (a horizontal band of the panel, in
/// window coordinates) shrunk by `scale`, its top left corner at `(dest_x, dest_y)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Strip {
    pub src: Rectangle,
    pub dest_x: f32,
    pub dest_y: f32,
    pub scale: f32,
}

/// Height of a strip: thin enough that the funnel's edges look curved, not stepped.
const STRIP_H: f32 = 4.0;
/// How narrow the panel gets where it enters the point (px).
const NECK: f32 = 24.0;
/// The two movements, as parts of the way in: first the funnel forms, then (overlapping it a
/// little) the panel slides down through it.
const BEND_UNTIL: f32 = 0.4;
const SLIDE_FROM: f32 = 0.25;

fn smoothstep(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The strips that draw `sheet` when it is `sucked` of the way into `target` (0: all there,
/// as laid out; 1: gone).
pub fn strips(sheet: Rectangle, target: Point, sucked: f32, strip_h: f32) -> Vec<Strip> {
    if sucked <= 0.0 {
        return vec![Strip { src: sheet, dest_x: sheet.x, dest_y: sheet.y, scale: 1.0 }];
    }
    if sucked >= 1.0 || sheet.width <= 0.0 || sheet.height <= 0.0 {
        return Vec::new();
    }
    let span = (target.y - sheet.y).max(1.0);
    let bend = smoothstep(sucked / BEND_UNTIL);
    let slide = ((sucked - SLIDE_FROM) / (1.0 - SLIDE_FROM)).clamp(0.0, 1.0).powi(2);
    let neck = (NECK / sheet.width).min(1.0);
    let centre = sheet.center_x();
    let mut y = sheet.y + slide * span;
    let mut out = Vec::new();
    let mut top = sheet.y;
    while top < sheet.y + sheet.height && y < target.y {
        let h = strip_h.min(sheet.y + sheet.height - top);
        // How far down toward the point this strip is now: the further, the narrower and the
        // nearer the point's side.
        let pull = bend * smoothstep((y - sheet.y) / span);
        let scale = 1.0 - pull * (1.0 - neck);
        let x = centre + pull * (target.x - centre);
        out.push(Strip { src: Rectangle::new(Point::new(sheet.x, top), Size::new(sheet.width, h)), dest_x: x - scale * sheet.width / 2.0, dest_y: y, scale });
        y += scale * h;
        top += h;
    }
    out
}

/// A panel placed like a bottom-centred container (`padding` kept clear around it) that pours
/// out of, or into, a point below the area: `target` from the area's bottom left corner.
pub struct Genie<'a, Message> {
    content: Element<'a, Message, Theme, Renderer>,
    sucked: f32,
    target: Vector,
    padding: Padding,
    /// A panel on its way out takes no input: what it would do is no longer on offer.
    interactive: bool,
}

impl<'a, Message> Genie<'a, Message> {
    pub fn new(content: impl Into<Element<'a, Message, Theme, Renderer>>, sucked: f32, target: Vector, padding: Padding, interactive: bool) -> Self {
        Self { content: content.into(), sucked, target, padding, interactive }
    }
}

impl<Message> Widget<Message, Theme, Renderer> for Genie<'_, Message> {
    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }
    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_ref(&self.content));
    }
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Fill)
    }

    fn layout(&mut self, tree: &mut Tree, renderer: &Renderer, limits: &layout::Limits) -> layout::Node {
        let area = limits.width(Length::Fill).height(Length::Fill).max();
        let room = Size::new((area.width - self.padding.left - self.padding.right).max(0.0), (area.height - self.padding.top - self.padding.bottom).max(0.0));
        let content = self.content.as_widget_mut().layout(&mut tree.children[0], renderer, &layout::Limits::new(Size::ZERO, room));
        let size = content.size();
        let at = Point::new(self.padding.left + (room.width - size.width) / 2.0, area.height - self.padding.bottom - size.height);
        layout::Node::with_children(area, vec![content.move_to(at)])
    }

    fn draw(&self, tree: &Tree, renderer: &mut Renderer, theme: &Theme, style: &renderer::Style, layout: Layout<'_>, cursor: mouse::Cursor, viewport: &Rectangle) {
        use iced::advanced::Renderer as _;
        let content = layout.children().next().expect("the content");
        if self.sucked <= 0.0 {
            self.content.as_widget().draw(&tree.children[0], renderer, theme, style, content, cursor, viewport);
            return;
        }
        let area = layout.bounds();
        let target = Point::new(area.x + self.target.x, area.y + area.height + self.target.y);
        let cursor = mouse::Cursor::Unavailable;
        for s in strips(content.bounds(), target, self.sucked, STRIP_H) {
            // A hair of overlap so no seam shows between strips.
            let clip = Rectangle::new(Point::new(s.dest_x, s.dest_y - 0.25), Size::new(s.src.width * s.scale, s.src.height * s.scale + 0.5));
            let place = Transformation::translate(s.dest_x, s.dest_y) * Transformation::scale(s.scale) * Transformation::translate(-s.src.x, -s.src.y);
            renderer.with_layer(clip, |r| {
                r.with_transformation(place, |r| {
                    self.content.as_widget().draw(&tree.children[0], r, theme, style, content, cursor, &s.src);
                });
            });
        }
    }

    fn update(&mut self, tree: &mut Tree, event: &Event, layout: Layout<'_>, cursor: mouse::Cursor, renderer: &Renderer, clipboard: &mut dyn Clipboard, shell: &mut Shell<'_, Message>, viewport: &Rectangle) {
        if !self.interactive {
            return;
        }
        let content = layout.children().next().expect("the content");
        self.content.as_widget_mut().update(&mut tree.children[0], event, content, cursor, renderer, clipboard, shell, viewport);
    }

    fn mouse_interaction(&self, tree: &Tree, layout: Layout<'_>, cursor: mouse::Cursor, viewport: &Rectangle, renderer: &Renderer) -> mouse::Interaction {
        if !self.interactive {
            return mouse::Interaction::None;
        }
        let content = layout.children().next().expect("the content");
        self.content.as_widget().mouse_interaction(&tree.children[0], content, cursor, viewport, renderer)
    }

    fn operate(&mut self, tree: &mut Tree, layout: Layout<'_>, renderer: &Renderer, operation: &mut dyn widget::Operation) {
        let content = layout.children().next().expect("the content");
        self.content.as_widget_mut().operate(&mut tree.children[0], content, renderer, operation);
    }

    fn overlay<'b>(&'b mut self, tree: &'b mut Tree, layout: Layout<'b>, renderer: &Renderer, viewport: &Rectangle, translation: Vector) -> Option<iced::advanced::overlay::Element<'b, Message, Theme, Renderer>> {
        if !self.interactive {
            return None;
        }
        let content = layout.children().next().expect("the content");
        self.content.as_widget_mut().overlay(&mut tree.children[0], content, renderer, viewport, translation)
    }
}

impl<'a, Message: 'a> From<Genie<'a, Message>> for Element<'a, Message, Theme, Renderer> {
    fn from(g: Genie<'a, Message>) -> Self {
        Element::new(g)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHEET: Rectangle = Rectangle { x: 300.0, y: 100.0, width: 600.0, height: 300.0 };
    const FACE: Point = Point { x: 60.0, y: 440.0 };

    fn centre(s: &Strip) -> f32 {
        s.dest_x + s.scale * s.src.width / 2.0
    }

    #[test]
    fn open_sheet_is_one_identity_strip() {
        assert_eq!(strips(SHEET, FACE, 0.0, 4.0), vec![Strip { src: SHEET, dest_x: SHEET.x, dest_y: SHEET.y, scale: 1.0 }]);
    }

    #[test]
    fn gone_sheet_draws_nothing() {
        assert!(strips(SHEET, FACE, 1.0, 4.0).is_empty());
    }

    #[test]
    fn half_way_funnels_toward_the_target() {
        let s = strips(SHEET, FACE, 0.5, 4.0);
        assert!(s.len() > 10, "{}", s.len());
        for w in s.windows(2) {
            assert!(w[1].scale <= w[0].scale, "narrower further down");
            assert!(centre(&w[1]) <= centre(&w[0]) && centre(&w[1]) >= FACE.x, "bending toward the face, never past it");
            assert!((w[1].dest_y - (w[0].dest_y + w[0].scale * w[0].src.height)).abs() < 0.01, "no gaps, no overlaps");
            assert_eq!(w[1].src.y, w[0].src.y + w[0].src.height, "the panel read top to bottom");
        }
        assert!(s.last().unwrap().scale < 0.6, "{}", s.last().unwrap().scale);
        assert!(s.iter().all(|x| x.dest_y < FACE.y), "nothing past the face");
        assert!(s[0].dest_y > SHEET.y, "already sliding in");
    }

    #[test]
    fn the_funnel_forms_before_the_panel_moves() {
        let s = strips(SHEET, FACE, 0.2, 4.0);
        assert_eq!(s[0].dest_y, SHEET.y, "the top still in place");
        assert!(s.last().unwrap().scale < s[0].scale, "while the bottom already narrows");
    }

    #[test]
    fn strips_stay_finite_for_any_target() {
        let narrow = Rectangle { width: 260.0, ..SHEET };
        for sheet in [SHEET, narrow] {
            for x in [-50.0, 60.0, 600.0, 2000.0] {
                for i in 0..10 {
                    let sucked = 0.05 + 0.1 * i as f32;
                    for s in strips(sheet, Point::new(x, 440.0), sucked, 4.0) {
                        assert!(s.dest_x.is_finite() && s.dest_y.is_finite(), "{s:?}");
                        assert!(s.scale > 0.0 && s.scale <= 1.0, "{s:?}");
                    }
                }
            }
        }
    }
}
