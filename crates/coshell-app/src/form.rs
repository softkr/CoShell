//! The dialogs' form language: bold section headings over rows of a label,
//! right-aligned against its control, with explanations lined up under the controls.

use iced::widget::{Space, column, container, pick_list, row, text};
use iced::{Alignment, Background, Border, Font, Length, Padding};

use crate::app::Message;
use crate::theme;
use crate::ui::{El, muted};

/// Space between a label and its control.
pub(crate) const GAP: f32 = 12.0;
/// Height of a dialog's title bar.
const TITLE_H: f32 = 40.0;

pub(crate) fn section<'a>(title: &'a str) -> El<'a> {
    text(title).size(12.5).font(Font { weight: iced::font::Weight::Bold, ..crate::fonts::SANS }).color(theme::TEXT).into()
}

/// A bold heading over its rows, which sit indented under it.
pub(crate) fn group<'a>(title: &'a str, rows: Vec<El<'a>>) -> El<'a> {
    column![section(title), column(rows).spacing(10).padding(Padding::ZERO.left(14.0))].spacing(10).into()
}

/// A label, right-aligned in a column `label_w` wide, against its control.
pub(crate) fn field<'a>(label_w: f32, label: &str, control: impl Into<El<'a>>) -> El<'a> {
    row![container(text(format!("{label}:")).size(12.5).color(theme::TEXT_2)).width(label_w).align_x(Alignment::End), control.into()]
        .spacing(GAP)
        .align_y(Alignment::Center)
        .into()
}

/// Something under a control (an explanation, a second line of controls), lined up with the controls.
pub(crate) fn under<'a>(label_w: f32, content: impl Into<El<'a>>) -> El<'a> {
    row![Space::new().width(label_w + GAP), content.into()].into()
}

/// A line of explanation under a control, no wider than `max_w`.
pub(crate) fn hint<'a>(label_w: f32, max_w: f32, s: impl Into<String>) -> El<'a> {
    under(label_w, container(muted(s)).max_width(max_w))
}

/// A choice shown in a drop-down by its label.
#[derive(Clone, Copy, PartialEq)]
pub(crate) struct Choice<T> {
    value: T,
    label: &'static str,
}

impl<T> std::fmt::Display for Choice<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label)
    }
}

/// One of `all`, picked from a drop-down `width` wide.
pub(crate) fn dropdown<'a, T: Copy + PartialEq + 'a>(all: &[T], current: T, name: fn(T) -> &'static str, msg: fn(T) -> Message, width: impl Into<Length>) -> El<'a> {
    let choices: Vec<Choice<T>> = all.iter().map(|&value| Choice { value, label: name(value) }).collect();
    pick_list(choices, Some(Choice { value: current, label: name(current) }), move |c: Choice<T>| msg(c.value))
        .text_size(12.5)
        .padding([6, 10])
        .width(width)
        .style(crate::ui::pick_style)
        .menu_style(crate::ui::menu_style)
        .into()
}

/// A dialog's title bar: the title centred over a hairline, rounded like the dialog it tops.
pub(crate) fn title_bar<'a>(title: impl text::IntoFragment<'a>) -> El<'a> {
    let bar = container(text(title).size(13).font(Font { weight: iced::font::Weight::Semibold, ..crate::fonts::SANS }).color(theme::TEXT))
        .center_x(Length::Fill)
        .center_y(TITLE_H)
        .style(|_| container::Style {
            background: Some(Background::Color(theme::BG_1)),
            // One less than the dialog's: the bar sits inside its outline.
            border: Border { radius: iced::border::Radius { top_left: crate::ui::R_DIALOG - 1.0, top_right: crate::ui::R_DIALOG - 1.0, ..Default::default() }, ..Default::default() },
            ..Default::default()
        });
    column![bar, crate::ui::rule(true)].into()
}
