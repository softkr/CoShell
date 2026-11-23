//! The status mark beside each AI reply in the chat: three dots that gather and scatter
//! while the AI thinks; a paper disc that folds in half over and over (down, across, up,
//! across) while it writes or runs a command; a blue dot for the newest reply; a grey one
//! for older replies.
//!
//! After ai_chat_status_indicator.html, without its turning disc: each fold flips the
//! whole disc to the paper's other side, so the next fold can start along the other axis
//! with nothing to turn. iced has no 3D transforms, so the flap's `rotateX` is drawn as a
//! half-ellipse squashed by the cosine of its angle.

use std::f32::consts::PI;

use iced::widget::canvas::{self, Frame, Geometry, Path, Stroke, gradient::Linear, path::arc::Elliptical};
use iced::widget::{Space, container};
use iced::{Background, Border, Color, Point, Radians, Rectangle, Renderer, Theme, Vector, mouse};

use crate::app::Message;
use crate::theme;
use crate::ui::El;

/// One cycle: four folds (down, across, up, across), ending as it began.
pub const CYCLE_SECS: f32 = 3.6;
const FOLD_SECS: f32 = 0.9;
/// Share of a fold spent folding; the rest it lies flat before the next one.
const FOLD_SHARE: f32 = 0.86;
/// CSS `cubic-bezier` control points of the fold: an even ease in and out, so the paper
/// keeps moving rather than snapping over and waiting.
const FOLD_EASE: [f32; 4] = [0.45, 0.0, 0.55, 1.0];
/// How much the flap's back darkens while it is in the air.
const BACK_SHADE: f32 = 0.14;

const BLUE_600: Color = Color::from_rgb(0x25 as f32 / 255.0, 0x63 as f32 / 255.0, 0xEB as f32 / 255.0);
const BLUE_500: Color = Color::from_rgb(0x3B as f32 / 255.0, 0x82 as f32 / 255.0, 0xF6 as f32 / 255.0);
const BLUE_200: Color = Color::from_rgb(0xBF as f32 / 255.0, 0xDB as f32 / 255.0, 0xFE as f32 / 255.0);
const SHADOW: Color = Color::from_rgb(0x0F as f32 / 255.0, 0x17 as f32 / 255.0, 0x2A as f32 / 255.0);
const SHADOW_FAR: Color = Color::from_rgb(0x1E as f32 / 255.0, 0x3A as f32 / 255.0, 0x8A as f32 / 255.0);
const CREASE: Color = Color::from_rgba(0x1E as f32 / 255.0, 0x40 as f32 / 255.0, 0xAF as f32 / 255.0, 0.28);

/// Where a reply stands.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Mark {
    /// The AI is thinking (nothing written yet): seconds into the cycle.
    Thinking(f32),
    /// The AI is writing or running a command: seconds into the fold cycle.
    Working(f32),
    /// The newest finished reply.
    Latest,
    /// An older reply.
    Past,
}

/// Diameter of the mark, the same in every state so it does not jump when a reply ends.
pub const DISC: f32 = 14.0;
/// How much nearer the lifted flap looks (CSS `perspective: 160px` on a 28px disc).
const PERSPECTIVE: f32 = 14.0 / 160.0;
/// The white centre over the dot's diameter.
const LATEST_EYE: f32 = 0.28;
const PAST_EYE: f32 = 0.21;

pub fn mark<'a>(m: Mark) -> El<'a> {
    match m {
        Mark::Thinking(t) => canvas::Canvas::new(Dots { t }).width(DISC).height(DISC).into(),
        Mark::Working(t) => canvas::Canvas::new(Fold { t }).width(DISC).height(DISC).into(),
        Mark::Latest => {
            let fill = iced::gradient::Linear::new(Radians(PI * 0.75)).add_stop(0.0, BLUE_600).add_stop(1.0, BLUE_500);
            dot(DISC, Background::Gradient(fill.into()), alpha(Color::WHITE, 0.9), LATEST_EYE)
        }
        Mark::Past => dot(DISC, Background::Color(theme::SLATE_600), alpha(Color::WHITE, 0.8), PAST_EYE),
    }
}

fn alpha(c: Color, a: f32) -> Color {
    Color { a, ..c }
}

/// A round mark with a smaller white disc in its centre.
fn dot<'a>(size: f32, fill: Background, eye: Color, eye_share: f32) -> El<'a> {
    let inner = size * eye_share;
    let centre = container(Space::new()).width(inner).height(inner).style(move |_| container::Style {
        background: Some(Background::Color(eye)),
        border: Border { radius: (inner / 2.0).into(), ..Default::default() },
        ..Default::default()
    });
    container(centre)
        .center(size)
        .style(move |_| container::Style { background: Some(fill), border: Border { radius: (size / 2.0).into(), ..Default::default() }, ..Default::default() })
        .into()
}

/// CSS `cubic-bezier(x1, y1, x2, y2)` at progress `x` (0..1).
fn ease(p: [f32; 4], x: f32) -> f32 {
    let curve = |a: f32, b: f32, s: f32| 3.0 * a * s * (1.0 - s).powi(2) + 3.0 * b * s * s * (1.0 - s) + s.powi(3);
    // x(s) rises monotonically for these control points: bisect for s, then read y(s).
    let (mut lo, mut hi) = (0.0_f32, 1.0_f32);
    for _ in 0..24 {
        let mid = (lo + hi) / 2.0;
        if curve(p[0], p[2], mid) < x { lo = mid } else { hi = mid }
    }
    curve(p[1], p[3], (lo + hi) / 2.0)
}

/// Value of keyframes `(share, value)` at share `x`, eased between neighbours.
fn keyframes(frames: &[(f32, f32)], x: f32) -> f32 {
    for w in frames.windows(2) {
        let ((x0, v0), (x1, v1)) = (w[0], w[1]);
        if x <= x1 {
            let s = if x1 > x0 { ((x - x0) / (x1 - x0)).clamp(0.0, 1.0) } else { 1.0 };
            return v0 + (v1 - v0) * ease(FOLD_EASE, s);
        }
    }
    frames.last().map_or(0.0, |f| f.1)
}

/// Which of the cycle's four folds is under way at `t`.
fn fold_index(t: f32) -> usize {
    (t.rem_euclid(CYCLE_SECS) / FOLD_SECS) as usize % 4
}

/// One fold at `t`: the flap's angle (0 flat on its half, π folded over), the light on it,
/// the shadow it casts.
fn fold(t: f32) -> (f32, f32, f32) {
    let x = (t / FOLD_SECS).rem_euclid(1.0);
    let angle = PI * ease(FOLD_EASE, (x / FOLD_SHARE).min(1.0));
    let sheen = keyframes(&[(0.0, 0.0), (0.38, 1.0), (FOLD_SHARE, 0.0)], x);
    let shadow = keyframes(&[(0.0, 0.0), (0.45, 0.95), (FOLD_SHARE, 0.0)], x);
    (angle, sheen, shadow)
}

/// Half of an ellipse centred on the origin: above the fold line (`up`) or below it.
fn half(rx: f32, ry: f32, up: bool) -> Path {
    Path::new(|b| {
        let (start, end) = if up { (PI, 2.0 * PI) } else { (0.0, PI) };
        b.ellipse(Elliptical { center: Point::ORIGIN, radii: Vector::new(rx, ry), rotation: Radians(0.0), start_angle: Radians(start), end_angle: Radians(end) });
        b.close();
    })
}

fn vertical(from: f32, to: f32) -> Linear {
    Linear::new(Point::new(0.0, from), Point::new(0.0, to))
}

/// The thinking dots gather and scatter once in this long; three times per cycle, with a
/// third of a turn each, so the cycle's end meets its start.
const GATHER_SECS: f32 = CYCLE_SECS / 3.0;
/// Radius of a dot when apart and when gathered into one, over the mark's radius.
const DOT_APART: f32 = 0.3;
const DOT_GATHERED: f32 = 0.6;
/// How far from the centre the dots scatter, over the mark's radius.
const SCATTER: f32 = 0.68;
const BLUE_400: Color = Color::from_rgb(0x60 as f32 / 255.0, 0xA5 as f32 / 255.0, 0xFA as f32 / 255.0);

/// How far apart the thinking dots are at `t` (0 gathered, 1 scattered), and their turn in radians.
fn gather(t: f32) -> (f32, f32) {
    let x = t / GATHER_SECS;
    let apart = 0.5 - 0.5 * (2.0 * PI * x).cos();
    (apart, x * 2.0 * PI / 3.0)
}

/// Three dots that gather into one and scatter again while the AI thinks.
struct Dots {
    t: f32,
}

impl canvas::Program<Message> for Dots {
    type State = ();

    fn draw(&self, _: &(), renderer: &Renderer, _: &Theme, bounds: Rectangle, _: mouse::Cursor) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let r = bounds.width.min(bounds.height) / 2.0;
        let (apart, spin) = gather(self.t);
        let dot = r * (DOT_GATHERED + (DOT_APART - DOT_GATHERED) * apart);
        // Keep the scattered dots inside the mark.
        let reach = (r * SCATTER).min(r - dot) * apart;
        frame.translate(Vector::new(bounds.width / 2.0, bounds.height / 2.0));
        for (k, colour) in [BLUE_600, BLUE_500, BLUE_400].into_iter().enumerate() {
            let angle = spin + k as f32 * 2.0 * PI / 3.0 - PI / 2.0;
            frame.fill(&Path::circle(Point::new(reach * angle.cos(), reach * angle.sin()), dot), colour);
        }
        vec![frame.into_geometry()]
    }
}

/// The folding paper disc at `t` seconds into its cycle.
struct Fold {
    t: f32,
}

impl canvas::Program<Message> for Fold {
    type State = ();

    fn draw(&self, _: &(), renderer: &Renderer, _: &Theme, bounds: Rectangle, _: mouse::Cursor) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let r = bounds.width.min(bounds.height) / 2.0;
        let (angle, sheen, shadow) = fold(self.t);
        let n = fold_index(self.t);
        // Each fold shows the paper's other side: blue to pale, then pale to blue.
        let (front, back) = if n % 2 == 0 { (BLUE_500, BLUE_200) } else { (BLUE_200, BLUE_500) };
        frame.translate(Vector::new(bounds.width / 2.0, bounds.height / 2.0));
        // The crease lies across for folds 0 and 2, upright for 1 and 3. Between folds the
        // disc is one colour all over, so the change of direction does not show.
        frame.rotate(Radians(n as f32 * PI / 2.0));
        // The half the flap leaves shows the other side; the half it lands on is shaded by it.
        frame.fill(&half(r, r, true), back);
        let lower = half(r, r, false);
        frame.fill(&lower, front);
        frame.fill(
            &lower,
            vertical(0.0, r)
                .add_stop(0.0, alpha(SHADOW, 0.32 * shadow))
                .add_stop(0.75, alpha(SHADOW_FAR, 0.08 * shadow))
                .add_stop(1.0, Color::TRANSPARENT),
        );
        // The flap: seen from the front while it rises, from the back once past upright.
        // Upright, its edge is nearest the eye and looks a little longer.
        let lift = angle.sin();
        let squash = angle.cos() / (1.0 - PERSPECTIVE * lift);
        if squash > 0.01 {
            let flap = half(r, r * squash, true);
            frame.fill(&flap, front);
            frame.fill(&flap, vertical(-r * squash, 0.0).add_stop(0.0, alpha(Color::WHITE, 0.42 * sheen)).add_stop(0.85, Color::TRANSPARENT));
        } else if squash < -0.01 {
            let flap = half(r, r * -squash, false);
            frame.fill(&flap, back);
            frame.fill(&flap, alpha(SHADOW, BACK_SHADE * lift));
        }
        // The crease shows only while the paper is bent.
        let line = Path::line(Point::new(-r * 0.96, 0.0), Point::new(r * 0.96, 0.0));
        frame.stroke(&line, Stroke::default().with_width(1.0).with_color(alpha(CREASE, CREASE.a * lift)));
        vec![frame.into_geometry()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thinking_dots_gather_scatter_and_loop() {
        assert!(gather(0.0).0.abs() < 1e-3, "gathered at the start");
        assert!((gather(GATHER_SECS / 2.0).0 - 1.0).abs() < 1e-3, "scattered half way");
        let (apart, spin) = gather(CYCLE_SECS);
        assert!(apart.abs() < 1e-3 && (spin - 2.0 * PI).abs() < 1e-3, "a full turn, gathered again, per cycle");
    }

    #[test]
    fn ease_matches_its_ends_and_rises() {
        assert!(ease(FOLD_EASE, 0.0).abs() < 1e-3);
        assert!((ease(FOLD_EASE, 1.0) - 1.0).abs() < 1e-3);
        assert!(ease(FOLD_EASE, 0.3) < ease(FOLD_EASE, 0.6));
    }

    #[test]
    fn folds_over_and_over_in_four_directions() {
        assert!(fold(0.0).0.abs() < 1e-3, "flat at the start");
        assert!((fold(FOLD_SECS * FOLD_SHARE).0 - PI).abs() < 1e-3, "folded over");
        assert!((fold(FOLD_SECS - 1e-3).0 - PI).abs() < 1e-3, "lies flat until the next fold");
        assert!(fold(FOLD_SECS + 1e-3).0 < 0.1, "the next fold starts flat again");
        let order: Vec<usize> = (0..5).map(|k| fold_index(k as f32 * FOLD_SECS + 0.01)).collect();
        assert_eq!(order, [0, 1, 2, 3, 0], "four directions, then round again");
    }
}
