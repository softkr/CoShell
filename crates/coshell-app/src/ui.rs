//! Views: header, tab bar, terminal / connect / failure bodies, status bar,
//! ⌘K palette, host-key and interactive-auth dialogs.
//!
//! Layout follows coshell_ai_native_ssh_terminal.html: header on top, server
//! sidebar · terminal workspace · AI chat in three columns, status bar below.

use crate::i18n::{tf, tr};
use coshell_ssh::{HostKeyDecision, HostKeyKind, HostKeyQuery, InteractiveQuery};
use iced::widget::{Space, button, center, column, container, mouse_area, opaque, row, scrollable, stack, text, text_input, tooltip};
use iced::{Alignment, Background, Border, Color, Element, Font, Length, Padding, Theme};
use zeroize::Zeroizing;

use crate::app::{CONFIRM_INPUT, CoShell, PALETTE_INPUT, Pending, Phase, RENAME_INPUT, SEARCH_INPUT, Search, Step, Tab, prompt_input_id};
use crate::app::{Message, PaletteItem};
use crate::icons::{ARROW_DOWN, ARROW_UP, ERASER, LOCK, LOCK_OPEN, SEARCH, X};
use crate::terminal::TerminalView;
use crate::hosts::Env;
use crate::theme::{self, alpha};
use crate::ui_hosts::env_color;

/// Top bar: session pill, safety lock, layout buttons. On macOS the window buttons sit in it,
/// so it is as tall as puts them on its centre line.
pub(crate) const HEADER_H: f32 = if cfg!(target_os = "macos") { 38.0 } else { 44.0 };
/// Session tab strip above the terminal.
pub(crate) const BAR_H: f32 = 44.0;
const TAB_MAX_W: f32 = 240.0;
/// Room a tab keeps beside its name: padding, dot, attention mark and close button.
const TAB_CHROME_W: f32 = 76.0;
/// A tab's rounded fill, inside the strip.
const TAB_H: f32 = 32.0;
const TAB_RENAME_W: f32 = 140.0;
const STATUSBAR_H: f32 = 28.0;
const MODAL_W: f32 = 520.0;
const PALETTE_W: f32 = 512.0;
const PALETTE_LIST_H: f32 = 288.0;
const SEARCH_BAR_INPUT_W: f32 = 240.0;
const SEARCH_BAR_TOP: f32 = 8.0;
const EXITED_BANNER_BOTTOM: f32 = 18.0;
/// Draggable strip between panes.
const DIVIDER_W: f32 = 4.0;
const TOAST_MAX_W: f32 = 560.0;
/// Above the status bar and the "session ended" banner.
const TOAST_BOTTOM: f32 = 44.0;

pub(crate) type El<'a> = Element<'a, Message>;

/// Corner radii: controls, small cards, large cards and dialogs.
pub(crate) const R_CONTROL: f32 = 8.0;
pub(crate) const R_CARD: f32 = 12.0;
pub(crate) const R_DIALOG: f32 = 16.0;
/// Tabs, sidebar rows, search fields.
pub(crate) const R_ROW: f32 = 9.0;
/// Vertical scrollbars: thin, neutral and trackless (iced's default is 10px with an indigo thumb).
const SCROLLBAR_W: f32 = 6.0;
const SCROLLBAR_MARGIN: f32 = 2.0;
/// Whiteness of the scroller: while scrolling, and under the pointer.
const SCROLLER: f32 = 0.18;
const SCROLLER_HOT: f32 = 0.35;
pub(crate) const R_PILL: f32 = 999.0;

pub(crate) const MEDIUM: Font = Font { weight: iced::font::Weight::Medium, ..crate::fonts::SANS };
pub(crate) const SEMIBOLD: Font = Font { weight: iced::font::Weight::Semibold, ..crate::fonts::SANS };

// ------------------------------------------------------------------ styles

#[derive(Clone, Copy)]
pub(crate) enum Btn {
    Primary,
    Plain,
    Ghost,
    Danger,
}

pub(crate) fn btn_style(kind: Btn) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let hover = matches!(status, button::Status::Hovered | button::Status::Pressed);
        let (bg, fg, line) = match kind {
            Btn::Primary => (if hover { alpha(theme::ACCENT, 0.88) } else { theme::ACCENT }, theme::ON_ACCENT, Color::TRANSPARENT),
            Btn::Danger => (theme::ERR, Color::WHITE, Color::TRANSPARENT),
            Btn::Plain => (if hover { alpha(Color::WHITE, 0.14) } else { theme::CHIP }, theme::TEXT, Color::TRANSPARENT),
            Btn::Ghost => (if hover { theme::BG_3 } else { Color::TRANSPARENT }, if hover { theme::TEXT } else { theme::TEXT_2 }, Color::TRANSPARENT),
        };
        let disabled = matches!(status, button::Status::Disabled);
        button::Style {
            background: Some(Background::Color(if disabled { alpha(bg, bg.a * 0.4) } else { bg })),
            text_color: if disabled { alpha(fg, 0.5) } else { fg },
            border: Border { color: line, width: 1.0, radius: R_CONTROL.into() },
            ..Default::default()
        }
    }
}

/// Tinted pill / chip: `c` at low opacity with a matching border (the mockup's `bg-x/10 border-x/30`).
pub(crate) fn tint_style(c: Color, on: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let hover = matches!(status, button::Status::Hovered | button::Status::Pressed);
        let (bg, line, fg) = if on {
            (alpha(c, if hover { 0.22 } else { 0.12 }), alpha(c, 0.32), c)
        } else {
            (if hover { theme::BG_3 } else { theme::BG_2 }, if hover { theme::LINE_2 } else { theme::LINE }, theme::TEXT_2)
        };
        button::Style {
            background: Some(Background::Color(bg)),
            text_color: fg,
            border: Border { color: line, width: 1.0, radius: R_PILL.into() },
            ..Default::default()
        }
    }
}

pub(crate) fn btn<'a>(label: impl Into<String>, kind: Btn, msg: Option<Message>) -> El<'a> {
    button(text(label.into()).size(13)).padding([5, 12]).style(btn_style(kind)).on_press_maybe(msg).into()
}

const ICON_SIZE: f32 = 14.0;
/// Icon beside a button's label.
const LABEL_ICON_SIZE: f32 = 12.0;

/// Colour of an icon on a `kind` button.
fn icon_color(kind: Btn, enabled: bool) -> Color {
    let color = match kind {
        Btn::Primary => theme::ON_ACCENT,
        Btn::Danger => Color::WHITE,
        Btn::Plain => theme::TEXT,
        Btn::Ghost => theme::TEXT_2,
    };
    if enabled { color } else { alpha(color, 0.4) }
}

/// A button with an icon before its label.
pub(crate) fn icon_text_btn<'a>(icon: &std::sync::LazyLock<iced::widget::svg::Handle>, label: impl Into<String>, kind: Btn, msg: Option<Message>) -> El<'a> {
    let color = icon_color(kind, msg.is_some());
    button(row![crate::icons::tinted(icon, LABEL_ICON_SIZE, color), text(label.into()).size(13)].spacing(6).align_y(Alignment::Center))
        .padding([5, 12])
        .style(btn_style(kind))
        .on_press_maybe(msg)
        .into()
}

/// A button showing only an icon; `label` appears on hover.
pub(crate) fn icon_btn<'a>(icon: &std::sync::LazyLock<iced::widget::svg::Handle>, label: &'a str, kind: Btn, msg: Option<Message>) -> El<'a> {
    let color = icon_color(kind, msg.is_some());
    let b = button(crate::icons::tinted(icon, ICON_SIZE, color)).padding(6).style(btn_style(kind)).on_press_maybe(msg);
    tip(b, label, tooltip::Position::Top)
}

/// Small label shown on hover.
pub(crate) fn tip<'a>(content: impl Into<El<'a>>, label: impl text::IntoFragment<'a>, pos: tooltip::Position) -> El<'a> {
    let bubble = container(text(label).size(11).color(theme::TEXT)).padding([4, 8]).style(|_| container::Style {
        background: Some(Background::Color(theme::BG_2)),
        border: Border { color: theme::LINE_2, width: 1.0, radius: R_CONTROL.into() },
        ..Default::default()
    });
    tooltip(content, bubble, pos).gap(4).into()
}

/// The scroll area that moved last (0: none lately). Its scrollbar shows until the app
/// hides it again, a moment after scrolling stops.
static SCROLLED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// How strongly that scrollbar shows, as the bits of an `f32` from 1 (shown) to 0 (faded away).
static SCROLLBAR_OPACITY: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Show `area`'s scrollbar (and no other) at `opacity`, or with area 0 hide them all.
pub(crate) fn show_scrollbar(area: u64, opacity: f32) {
    SCROLLED.store(area, std::sync::atomic::Ordering::Relaxed);
    SCROLLBAR_OPACITY.store(opacity.to_bits(), std::sync::atomic::Ordering::Relaxed);
}

/// Change how strongly the scrollbar now showing shows (it fades out).
pub(crate) fn fade_scrollbar(opacity: f32) {
    SCROLLBAR_OPACITY.store(opacity.to_bits(), std::sync::atomic::Ordering::Relaxed);
}

/// A vertically scrolling area with the app's thin scrollbar, which shows only while
/// the area is being scrolled (or the bar itself is under the pointer).
#[track_caller]
pub(crate) fn scroll<'a>(content: impl Into<El<'a>>) -> scrollable::Scrollable<'a, Message> {
    use std::hash::{Hash, Hasher};
    // Each place in the code that makes a scroll area is one area.
    let at = std::panic::Location::caller();
    let mut h = std::hash::DefaultHasher::new();
    (at.file(), at.line(), at.column()).hash(&mut h);
    let area = h.finish().max(1);
    let bar = scrollable::Scrollbar::new().width(SCROLLBAR_W).scroller_width(SCROLLBAR_W).margin(SCROLLBAR_MARGIN);
    scrollable(content).direction(scrollable::Direction::Vertical(bar)).on_scroll(move |_| Message::Scrolled(area)).style(move |t, st| scroll_style(t, st, area))
}

fn scroll_style(theme: &Theme, status: scrollable::Status, area: u64) -> scrollable::Style {
    let hot = matches!(
        status,
        scrollable::Status::Hovered { is_vertical_scrollbar_hovered: true, .. } | scrollable::Status::Dragged { is_vertical_scrollbar_dragged: true, .. }
    );
    let shown = if SCROLLED.load(std::sync::atomic::Ordering::Relaxed) == area { f32::from_bits(SCROLLBAR_OPACITY.load(std::sync::atomic::Ordering::Relaxed)) } else { 0.0 };
    let rail = scrollable::Rail {
        background: None,
        border: Border::default(),
        scroller: scrollable::Scroller {
            background: Background::Color(alpha(Color::WHITE, if hot { SCROLLER_HOT } else { SCROLLER * shown })),
            border: Border { radius: R_PILL.into(), ..Default::default() },
        },
    };
    scrollable::Style { vertical_rail: rail, horizontal_rail: rail, ..scrollable::default(theme, status) }
}

pub(crate) fn card_style(border_color: Color) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(Background::Color(theme::BG_2)),
        border: Border { color: border_color, width: 1.0, radius: R_DIALOG.into() },
        text_color: Some(theme::TEXT),
        shadow: iced::Shadow {
            color: Color::from_rgba(0.0, 0.0, 0.0, 0.5),
            offset: iced::Vector::new(0.0, 20.0),
            blur_radius: 60.0,
        },
        ..Default::default()
    }
}

/// Drop-down in the app's colours (the stock one is grey with a heavy arrow).
pub(crate) fn pick_style(_: &Theme, status: iced::widget::pick_list::Status) -> iced::widget::pick_list::Style {
    use iced::widget::pick_list::Status;
    let hover = matches!(status, Status::Hovered | Status::Opened { .. });
    iced::widget::pick_list::Style {
        text_color: theme::TEXT,
        placeholder_color: theme::TEXT_3,
        handle_color: theme::TEXT_2,
        background: Background::Color(if hover { theme::BG_3 } else { theme::BG_2 }),
        border: Border { color: if hover { theme::LINE_2 } else { theme::LINE }, width: 1.0, radius: R_CONTROL.into() },
    }
}

pub(crate) fn menu_style(_: &Theme) -> iced::overlay::menu::Style {
    iced::overlay::menu::Style {
        background: Background::Color(theme::BG_2),
        border: Border { color: theme::LINE_2, width: 1.0, radius: R_CONTROL.into() },
        text_color: theme::TEXT_2,
        selected_text_color: theme::TEXT,
        selected_background: Background::Color(theme::BG_3),
        shadow: iced::Shadow { color: Color::from_rgba(0.0, 0.0, 0.0, 0.4), offset: iced::Vector::new(0.0, 8.0), blur_radius: 24.0 },
    }
}

pub(crate) fn input_style(_: &Theme, status: text_input::Status) -> text_input::Style {
    let focused = matches!(status, text_input::Status::Focused { .. });
    text_input::Style {
        background: Background::Color(theme::BG_0),
        border: Border { color: if focused { alpha(theme::ACCENT, 0.6) } else { theme::LINE }, width: 1.0, radius: R_CONTROL.into() },
        icon: theme::TEXT_3,
        placeholder: theme::TEXT_3,
        value: theme::TEXT,
        selection: alpha(theme::ACCENT, 0.35),
    }
}

/// Text field without a box (the palette's and the terminal search's).
pub(crate) fn bare_input_style(_: &Theme, _: text_input::Status) -> text_input::Style {
    text_input::Style {
        background: Background::Color(Color::TRANSPARENT),
        border: Border::default(),
        icon: theme::TEXT_3,
        placeholder: theme::TEXT_3,
        value: theme::TEXT,
        selection: alpha(theme::ACCENT, 0.35),
    }
}

pub(crate) fn fill(c: Color) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style { background: Some(Background::Color(c)), ..Default::default() }
}

/// 1px rule across (`true`) or down.
pub(crate) fn rule<'a>(across: bool) -> El<'a> {
    let c = container(Space::new()).style(fill(theme::LINE));
    if across { c.height(1).width(Length::Fill).into() } else { c.width(1).height(Length::Fill).into() }
}

/// Round status light.
pub(crate) fn dot<'a>(c: Color, size: f32) -> El<'a> {
    container(Space::new()).width(size).height(size).style(move |_| container::Style {
        background: Some(Background::Color(c)),
        border: Border { radius: (size / 2.0).into(), ..Default::default() },
        ..Default::default()
    })
    .into()
}

/// Keyboard hint (`⌘K`, `Return ↵`).
pub(crate) fn kbd<'a>(s: impl Into<String>) -> El<'a> {
    container(mono(s, 10.0, theme::TEXT_2)).padding([1, 5]).style(|_| container::Style {
        background: Some(Background::Color(theme::BG_0)),
        border: Border { color: theme::LINE, width: 1.0, radius: 4.0.into() },
        ..Default::default()
    })
    .into()
}

/// The assistant's mark: a small iridescent disc (blue through purple to pink).
/// The AI's expression: eyes shut for a blink, or glancing aside while it works.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Face {
    pub blink: bool,
    /// -1 looks left, 1 right, 0 straight ahead.
    pub look: i8,
}

impl Face {
    /// Eyes open, looking ahead: for faces that stay still (old messages, block badges).
    pub const STILL: Face = Face { blink: false, look: 0 };

    /// Blinks, but keeps looking ahead (the AI is not working on anything).
    pub fn idle(self) -> Face {
        Face { look: 0, ..self }
    }
}

/// Pill width over its height.
const FACE_ASPECT: f32 = 1.8;
/// Eye diameter over the pill's height.
const FACE_EYE: f32 = 0.32;
/// Eye height while blinking, over its diameter.
const FACE_BLINK: f32 = 0.3;
/// Gap between the eyes, over their diameter.
const FACE_EYE_GAP: f32 = 0.9;
/// How far the eyes move on a glance, over their diameter.
const FACE_LOOK: f32 = 0.45;

/// The AI's mark: a white pill with two eyes, `height` tall.
pub(crate) fn face<'a>(height: f32, f: Face) -> El<'a> {
    let eye = (height * FACE_EYE).max(2.0);
    let eye_h = if f.blink { (eye * FACE_BLINK).max(1.0) } else { eye };
    let shift = f32::from(f.look) * eye * FACE_LOOK;
    let pupil = move || {
        container(Space::new()).width(eye).height(eye_h).style(move |_| container::Style {
            background: Some(Background::Color(theme::BG_0)),
            border: Border { radius: (eye / 2.0).into(), ..Default::default() },
            ..Default::default()
        })
    };
    let eyes = row![pupil(), pupil()].spacing(eye * FACE_EYE_GAP).align_y(Alignment::Center);
    // Padding on one side moves the centred eyes by `shift`.
    let aside = Padding { left: (2.0 * shift).max(0.0), right: (-2.0 * shift).max(0.0), ..Padding::ZERO };
    container(container(eyes).padding(aside).center(Length::Fill))
        .width(height * FACE_ASPECT)
        .height(height)
        .style(move |_| container::Style {
            background: Some(Background::Color(theme::TEXT)),
            border: Border { radius: (height / 2.0).into(), ..Default::default() },
            ..Default::default()
        })
        .into()
}

pub(crate) fn mono<'a>(s: impl Into<String>, size: f32, color: Color) -> El<'a> {
    text(s.into()).size(size).font(crate::fonts::MONO).color(color).into()
}

pub(crate) fn muted<'a>(s: impl Into<String>) -> El<'a> {
    text(s.into()).size(11).color(theme::TEXT_3).into()
}

// ------------------------------------------------------------------ root

/// Draggable divider between panes: a 1px border on the terminal's side of a wider grab strip.
fn divider<'a>(on_press: Message, line_right: bool, color: Color) -> El<'a> {
    let strip = container(Space::new()).width(DIVIDER_W - 1.0).height(Length::Fill).style(fill(color));
    // The hairline is translucent: give it an opaque base, or beside the see-through
    // sidebar it shows the desktop behind the window (a bright white line over a light one).
    let line = || container(rule(false)).height(Length::Fill).style(fill(theme::BG_0));
    let content: El<'a> = if line_right { row![strip, line()].into() } else { row![line(), strip].into() };
    mouse_area(content).on_press(on_press).interaction(iced::mouse::Interaction::ResizingHorizontally).into()
}

pub fn view(app: &CoShell) -> El<'_> {
    // Toolbar across the top, then sidebar · terminal, then the status bar.
    // Everything but the sidebar is opaque: the window itself is see-through on macOS.
    let work = container(column![tab_bar(app), body(app)].width(Length::Fill)).width(Length::Fill).height(Length::Fill).style(fill(theme::BG_0));
    let mut cols = row![].height(Length::Fill);
    if app.settings.sidebar_open {
        cols = cols.push(crate::ui_sidebar::sidebar(app)).push(divider(Message::SidebarResizeStart, true, theme::SIDEBAR));
    }
    let mut main = column![header(app), cols.push(work)];
    if app.settings.statusbar_open {
        main = main.push(status_bar(app));
    }
    let main: El<'_> = main.into();
    let mut layers = vec![main];
    if let Some(n) = &app.notice {
        layers.push(toast(n));
    }
    if app.tunnel_panel {
        layers.push(crate::ui_tunnel::popover(app));
    }
    // Below the server editor, which the settings dialog opens.
    if let Some(d) = &app.settings_draft {
        layers.push(modal(crate::ui_settings::dialog(app, d)));
    }
    if let Some(k) = &app.key_import {
        layers.push(modal(crate::ui_keys::import_dialog(k, app.composing)));
    }
    if let Some(ed) = &app.snippet_editor {
        layers.push(modal(crate::ui_sidebar::snippet_editor(ed)));
    }
    if let Some(ed) = &app.editor {
        layers.push(modal(crate::ui_hosts::editor(app, ed)));
    }
    if let Some((id, a)) = &app.attach_confirm {
        let name = app.tabs.iter().find(|t| t.id == *id).map_or("", |t| t.display_name());
        layers.push(modal(crate::ui_hosts::attach_confirm(app, name, a)));
    }
    if let Some(tab) = app.active_tab()
        && let Some(p) = &tab.prompt
    {
        layers.push(modal(match p {
            Pending::HostKey { query, confirm, .. } => hostkey_dialog(tab, query, confirm),
            Pending::Interactive { query, answers, save, .. } => interactive_dialog(tab, query, answers, *save),
        }));
    }
    if let Some(tab) = app.close_confirm.and_then(|id| app.tabs.iter().find(|t| t.id == id)) {
        layers.push(modal(close_dialog(tab)));
    }
    if let Some(alias) = &app.delete_confirm {
        layers.push(modal(delete_dialog(app, alias)));
    }
    if let Some(tab) = app.active_tab()
        && let Some(c) = &tab.files.confirm
    {
        layers.push(modal(crate::ui_files::confirm_dialog(tab, c)));
    }
    if app.palette.is_some() {
        layers.push(palette(app));
    }
    if app.about {
        layers.push(modal(about_dialog(app)));
    }
    if let (true, Some(u)) = (app.update_confirm, &app.update) {
        layers.push(modal(update_dialog(u, app.update_busy)));
    }
    // Korean (and other IME) composing text drawn inline, without the runtime's boxed overlay.
    crate::ime::Ime::new(stack(layers), theme::TEXT, theme::BG_0).on_compose(Message::Composing).into()
}

/// Dismissible notice above the status bar (also goes away by itself).
fn toast(message: &str) -> El<'_> {
    let card = container(row![dot(theme::OK, 6.0), text(message).size(12).color(theme::SLATE_100)].spacing(8).align_y(Alignment::Center))
        .padding([8, 14])
        .max_width(TOAST_MAX_W)
        .style(|_| container::Style {
            background: Some(Background::Color(theme::BG_2)),
            border: Border { color: theme::LINE_2, width: 1.0, radius: R_CARD.into() },
            shadow: iced::Shadow { color: Color::from_rgba(0.0, 0.0, 0.0, 0.45), offset: iced::Vector::new(0.0, 10.0), blur_radius: 30.0 },
            ..Default::default()
        });
    let card = mouse_area(card).on_press(Message::NoticeDismiss);
    container(opaque(card)).center_x(Length::Fill).align_bottom(Length::Fill).padding(Padding::ZERO.bottom(TOAST_BOTTOM)).into()
}

pub(crate) fn modal(content: El<'_>) -> El<'_> {
    opaque(container(center(content)).width(Length::Fill).height(Length::Fill).style(|_| container::Style {
        background: Some(Background::Color(theme::SCRIM)),
        ..Default::default()
    }))
}

// ------------------------------------------------------------------ chrome

/// The layout buttons' pictures: a window outline with the panel's part filled while it shows.
#[derive(Clone, Copy, PartialEq, Eq)]
enum LayoutIcon {
    Left,
}

const LAYOUT_ICON: (f32, f32) = (17.0, 15.0);
const LAYOUT_LINE: f32 = 1.5;
/// Gap between the outline and the filled panel part.
const LAYOUT_GAP: f32 = 1.5;
/// Width (or height) of the filled panel part.
const LAYOUT_PANE: f32 = 4.0;

fn layout_icon<'a>(kind: LayoutIcon, on: bool, color: Color) -> El<'a> {
    let (w, h) = LAYOUT_ICON;
    let pane = |w: Length, h: Length, filled: bool, radius: iced::border::Radius| {
        container(Space::new()).width(w).height(h).style(move |_| container::Style {
            background: filled.then_some(Background::Color(color)),
            border: Border { radius, ..Default::default() },
            ..Default::default()
        })
    };
    let r = |tl: f32, tr: f32, br: f32, bl: f32| iced::border::Radius { top_left: tl, top_right: tr, bottom_right: br, bottom_left: bl };
    let outline = |content: El<'a>, w: f32, h: f32| -> El<'a> {
        container(content)
            .width(w)
            .height(h)
            .padding(LAYOUT_LINE + LAYOUT_GAP)
            .style(move |_| container::Style {
                border: Border { color, width: LAYOUT_LINE, radius: 3.5.into() },
                ..Default::default()
            })
            .into()
    };
    if !on {
        // Closed: just the divider where the panel would be.
        let divider = container(Space::new()).width(LAYOUT_LINE).height(Length::Fill).style(move |_| container::Style {
            background: Some(Background::Color(color)),
            ..Default::default()
        });
        let edge = Space::new().width(LAYOUT_GAP + LAYOUT_PANE + LAYOUT_GAP / 2.0);
        let content: El<'a> = match kind {
            LayoutIcon::Left => row![edge, divider].into(),
        };
        return container(content)
            .width(w)
            .height(h)
            .padding(LAYOUT_LINE)
            .style(move |_| container::Style {
                border: Border { color, width: LAYOUT_LINE, radius: 3.5.into() },
                ..Default::default()
            })
            .into();
    }
    match kind {
        LayoutIcon::Left => outline(row![pane(LAYOUT_PANE.into(), Length::Fill, on, r(1.0, 0.0, 0.0, 1.0))].into(), w, h),
    }
}

fn layout_btn<'a>(kind: LayoutIcon, on: bool, label: &'a str, msg: Message) -> El<'a> {
    let b = button(container(layout_icon(kind, on, theme::SLATE_300)).center(28)).padding(0).on_press(msg).style(|_, st| button::Style {
        background: matches!(st, button::Status::Hovered | button::Status::Pressed).then_some(Background::Color(theme::BG_3)),
        border: Border { radius: 6.0.into(), ..Default::default() },
        ..Default::default()
    });
    tip(b, label, tooltip::Position::Bottom)
}

/// Room for macOS's window buttons, which sit on the header.
const TRAFFIC_LIGHTS_W: f32 = if cfg!(target_os = "macos") { 72.0 } else { 0.0 };

/// Top bar: the search pill and safety lock · layout button.
fn header(app: &CoShell) -> El<'_> {
    let session = button(
        row![
            crate::icons::tinted(&SEARCH, 12.0, theme::TEXT_2),
            text(tr("서버 검색 · 명령")).size(11.5).color(theme::TEXT_2).wrapping(text::Wrapping::None),
            container(mono("⌘K", 10.0, theme::TEXT_3)).padding([1, 5]).style(|_| container::Style {
                background: Some(Background::Color(alpha(Color::WHITE, 0.06))),
                border: Border { radius: 5.0.into(), ..Default::default() },
                ..Default::default()
            }),
        ]
        .spacing(8)
        .align_y(Alignment::Center),
    )
    .padding(Padding { top: 5.0, right: 6.0, bottom: 5.0, left: 12.0 })
    .on_press(Message::OpenPalette)
    .style(|_, st| {
        let hover = matches!(st, button::Status::Hovered | button::Status::Pressed);
        button::Style {
            background: Some(Background::Color(if hover { theme::BG_3 } else { theme::BG_2 })),
            text_color: theme::SLATE_300,
            border: Border { color: theme::LINE, width: 1.0, radius: R_PILL.into() },
            ..Default::default()
        }
    });
    let guard_on = app.prod_guard;
    let guard = button(
        row![
            crate::icons::tinted(if guard_on { &LOCK } else { &LOCK_OPEN }, 14.0, if guard_on { theme::AMBER_300 } else { theme::TEXT_2 }),
            text(if guard_on { tr("운영 안전 잠금 ON") } else { tr("운영 안전 잠금 OFF") }).size(11).font(MEDIUM),
        ]
        .spacing(6)
        .align_y(Alignment::Center),
    )
    .padding([4, 10])
    .on_press(Message::ProdGuard)
    .style(move |_, st| {
        let hover = matches!(st, button::Status::Hovered | button::Status::Pressed);
        let (bg, line, fg) = if guard_on {
            (alpha(theme::WARN_500, if hover { 0.2 } else { 0.1 }), alpha(theme::WARN_500, 0.3), theme::AMBER_300)
        } else {
            (if hover { theme::BG_3 } else { theme::BG_2 }, theme::LINE, theme::TEXT_2)
        };
        button::Style {
            background: Some(Background::Color(bg)),
            text_color: fg,
            border: Border { color: line, width: 1.0, radius: R_PILL.into() },
            ..Default::default()
        }
    });
    let mut center = row![tip(session, tr("서버 전환 · 접속 (⌘K)"), tooltip::Position::Bottom)].spacing(8).align_y(Alignment::Center);
    // The lock only applies to production sessions, so it only shows on one.
    if app.active_tab().is_some_and(|t| t.env == Env::Prod) {
        center = center.push(tip(guard, tr("운영 서버 위험 명령어 실행 보호 모드"), tooltip::Position::Bottom));
    }

    let side = app.settings.sidebar_open;
    let left = layout_btn(LayoutIcon::Left, side, if side { tr("서버 사이드바 숨기기 (⌘B)") } else { tr("서버 사이드바 보기 (⌘B)") }, Message::SidebarToggle);

    let bar = stack![
        container(row![left].align_y(Alignment::Center))
            .padding(Padding { top: 0.0, right: 12.0, bottom: 0.0, left: 12.0 + TRAFFIC_LIGHTS_W })
            .width(Length::Fill)
            .height(HEADER_H - 1.0)
            .center_y(HEADER_H - 1.0),
        container(center).center_x(Length::Fill).center_y(HEADER_H - 1.0),
    ];
    let bar = container(column![bar, rule(true)]).width(Length::Fill).style(fill(theme::BG_1));
    // Buttons capture their clicks; only empty space reaches this.
    mouse_area(bar).on_press(Message::WindowDrag).on_double_click(Message::WindowZoom).into()
}

/// Env shown before an active tab's name ("운영 · app-01").
fn tab_group(t: &Tab) -> Option<&'static str> {
    match t.env {
        Env::None if t.is_local() => Some(tr("로컬")),
        Env::None => None,
        e => Some(e.label()),
    }
}

/// Dot colour of a tab, by session state: connected green, running a command amber,
/// disconnected red (grey while still connecting).
pub(crate) fn tab_color(t: &Tab) -> Color {
    match (&t.phase, t.exited) {
        (Phase::Failed(_), _) | (Phase::Live(_), true) => theme::ERR_TX,
        (Phase::Connecting, _) => theme::TEXT_3,
        _ if t.metas.last().is_some_and(|m| m.took.is_none()) => theme::WARN,
        _ => theme::OK,
    }
}

/// One session tab: coloured top edge when active, `env · name`, close.
fn tab_item<'a>(app: &'a CoShell, t: &'a Tab) -> El<'a> {
    let active = Some(t.id) == app.active && !app.host_view;
    let attention = t.prompt.is_some() && !active;
    let title: El<'a> = match &app.rename {
        Some((id, name)) if *id == t.id => text_input(tr("탭 이름"), name)
            .id(RENAME_INPUT)
            .on_input(Message::TabRename)
            .on_submit(Message::TabRenameCommit)
            .size(12)
            .padding([1, 5])
            .width(TAB_RENAME_W)
            .style(input_style)
            .into(),
        _ => {
            let mut r = row![].align_y(Alignment::Center);
            if active && let Some(g) = tab_group(t) {
                r = r.push(text(g).size(12).font(MEDIUM).color(theme::TEXT_2).wrapping(text::Wrapping::None));
                r = r.push(text(" · ").size(12).font(MEDIUM).color(theme::TEXT_2).wrapping(text::Wrapping::None));
            }
            r.push(
                text(t.display_name().to_string())
                    .size(12)
                    .font(crate::fonts::weight(crate::fonts::MONO, iced::font::Weight::Medium))
                    .color(if active { theme::TEXT } else { theme::TEXT_2 })
                    .wrapping(text::Wrapping::None),
            )
            .into()
        }
    };
    let c = tab_color(t);
    let mut label = row![dot(if active { c } else { alpha(c, 0.6) }, 8.0), container(title).max_width(TAB_MAX_W - TAB_CHROME_W).clip(true)].spacing(8).align_y(Alignment::Center);
    if attention {
        label = label.push(text("!").size(11).color(theme::WARN));
    }
    if app.tabs.len() > 1 {
        label = label.push(tip(
            button(crate::icons::tinted(&X, 12.0, theme::TEXT_3)).padding(2).style(btn_style(Btn::Ghost)).on_press(Message::Close(t.id)),
            tr("탭 닫기"),
            tooltip::Position::Bottom,
        ));
    }
    let content = container(label).padding(Padding { top: 0.0, right: 8.0, bottom: 0.0, left: 10.0 }).height(Length::Fill).center_y(Length::Fill);
    let tab = button(content).padding(0).height(TAB_H).on_press(Message::Activate(t.id)).style(move |_, status| {
        let hover = matches!(status, button::Status::Hovered | button::Status::Pressed);
        button::Style {
            background: Some(Background::Color(if active {
                alpha(Color::WHITE, 0.11)
            } else if hover {
                alpha(Color::WHITE, 0.075)
            } else {
                Color::TRANSPARENT
            })),
            text_color: theme::TEXT,
            border: Border { color: if active { theme::LINE } else { Color::TRANSPARENT }, width: 1.0, radius: R_ROW.into() },
            ..Default::default()
        }
    });
    container(tab).max_width(TAB_MAX_W).into()
}

/// Session tabs on the left; AI check, find, export and clear on the right.
fn tab_bar(app: &CoShell) -> El<'_> {
    let mut tabs = row(app.tabs.iter().map(|t| tab_item(app, t)).collect::<Vec<_>>())
        .spacing(6)
        .padding([0, 10])
        .height(BAR_H - 1.0)
        .align_y(Alignment::Center);
    tabs = tabs.push(tip(
        button(container(crate::icons::tinted(&crate::icons::PLUS, 15.0, theme::TEXT_2)).center(TAB_H))
            .padding(0)
            .on_press(Message::OpenPalette)
            .style(|_, st| button::Style {
                background: matches!(st, button::Status::Hovered).then_some(Background::Color(alpha(theme::BG_3, 0.5))),
                border: Border { radius: R_CONTROL.into(), ..Default::default() },
                ..Default::default()
            }),
        tr("새 세션 열기"),
        tooltip::Position::Bottom,
    ));
    let strip = scrollable(tabs)
        .direction(scrollable::Direction::Horizontal(scrollable::Scrollbar::hidden()))
        .width(Length::Fill)
        .height(BAR_H - 1.0);

    let live = app.live_tab().is_some() && !app.host_view;
    let diagnose = button(
        row![
            face(12.0, if app.active_tab().is_some_and(|t| t.chat.is_streaming()) { app.face } else { app.face.idle() }),
            text(tr("AI 진단")).size(11.5).font(MEDIUM)
        ]
        .spacing(6)
        .align_y(Alignment::Center),
    )
    .padding([6, 11])
    .on_press_maybe(app.active_tab().map(|_| Message::AiDiagnose))
    .style(|_, st| {
        let hover = matches!(st, button::Status::Hovered | button::Status::Pressed);
        button::Style {
            background: Some(Background::Color(alpha(theme::ACCENT, if hover { 0.2 } else { 0.12 }))),
            text_color: theme::ACCENT_TX,
            border: Border { radius: R_PILL.into(), ..Default::default() },
            ..Default::default()
        }
    });
    let tool = |icon: &'static std::sync::LazyLock<iced::widget::svg::Handle>, label: &'static str, msg: Message| -> El<'static> {
        let b = button(crate::icons::tinted(icon, 14.0, theme::TEXT_2)).padding(6).on_press_maybe(live.then_some(msg)).style(|_, st| button::Style {
            background: matches!(st, button::Status::Hovered).then_some(Background::Color(theme::BG_3)),
            border: Border { radius: 6.0.into(), ..Default::default() },
            ..Default::default()
        });
        tip(b, label, tooltip::Position::Bottom)
    };
    let tools = row![
        tip(diagnose, tr("현재 서버 AI 자동 진단"), tooltip::Position::Bottom),
        tool(&SEARCH, tr("터미널 출력 검색 (⌘F)"), Message::Shortcut(crate::app::Shortcut::Find)),
        tool(&ERASER, tr("터미널 화면 지우기 (clear)"), Message::TermClear),
    ]
    .spacing(4)
    .align_y(Alignment::Center);
    let mut bar = row![strip].height(BAR_H - 1.0).align_y(Alignment::Center);
    if app.active_tab().is_some() && !app.host_view {
        bar = bar
            .push(container(tools).padding(Padding { top: 0.0, right: 8.0, bottom: 0.0, left: 8.0 }).center_y(BAR_H - 1.0));
    }
    container(column![bar, rule(true)]).width(Length::Fill).style(fill(theme::BG_1)).into()
}

/// `user@name (address)` of a session's target.
fn target_line(t: &Tab) -> String {
    let last = t.route.last().cloned().unwrap_or_default();
    match last.split_once('@') {
        Some((user, host)) => format!("{user}@{} ({})", t.name, host),
        None if last.is_empty() => t.name.clone(),
        None => format!("{} ({last})", t.name),
    }
}

fn status_bar(app: &CoShell) -> El<'_> {
    let sep = || -> El<'_> { container(Space::new()).width(1).height(10).style(fill(theme::LINE_2)).into() };
    let small = |s: String, c: Color| -> El<'_> { text(s).size(11).color(c).into() };
    let mut left: Vec<El<'_>> = Vec::new();
    let mut right: Vec<El<'_>> = Vec::new();
    match app.active_tab() {
        Some(t) => {
            let (state, color) = match (&t.phase, t.exited) {
                (Phase::Connecting, _) => (tr("연결 중"), theme::WARN),
                (Phase::Failed(_), _) => (tr("연결 실패"), theme::ERR_TX),
                (Phase::Live(_), true) => (tr("연결 종료됨"), theme::ERR_TX),
                (Phase::Live(_), false) if t.is_local() => (tr("로컬 셸"), theme::OK),
                (Phase::Live(_), false) => (tr("SSH 연결됨"), theme::OK),
            };
            left.push(row![dot(color, 7.0), text(state).size(11).font(MEDIUM).color(color)].spacing(6).align_y(Alignment::Center).into());
            left.push(sep());
            if t.is_local() {
                left.push(small(format!("{}@{} (localhost)", crate::login_user(), coshell_term::local_shell_name()), theme::SLATE_300));
            } else {
                left.push(small(target_line(t), theme::SLATE_300));
                // Jump hosts and tunnels are details: shown only when there are some.
                if t.route.len() > 1 {
                    left.push(small(tf("경유 {0}", &[&(t.route.len() - 1)]), theme::TEXT_3));
                }
                if !t.tunnels.is_empty() {
                    let (label, failed) = crate::ui_tunnel::summary(t);
                    let mut content = row![crate::icons::icon(&crate::icons::PORT_FORWARDING, 13.0), text(label).size(11).color(theme::TEXT_2)]
                        .spacing(5)
                        .align_y(Alignment::Center);
                    if failed > 0 {
                        content = content.push(text(tf("오류 {0}", &[&failed])).size(11).color(theme::ERR));
                    }
                    left.push(button(content).padding([1, 6]).style(btn_style(Btn::Ghost)).on_press(Message::TunnelPanel).into());
                }
                // Files on their way, whichever sidebar page is showing.
                if t.files.running() > 0 {
                    left.push(small(tf("파일 전송 중 {0}", &[&t.files.running()]), theme::ACCENT_TX));
                }
            }
            if !t.is_local() {
                let ms = t.stats.as_ref().and_then(|s| s.latency_ms).map(|ms| format!("{ms}ms")).unwrap_or_else(|| {
                    let total: std::time::Duration = t.steps.iter().filter_map(|s| if let Step::Done(d) = s { Some(*d) } else { None }).sum();
                    format!("{}ms", total.as_millis())
                });
                right.push(row![small(tr("지연시간: ").into(), theme::TEXT_2), small(ms, theme::OK)].into());
                right.push(sep());
            }
            right.push(small("UTF-8".into(), theme::TEXT_2));
            if let Some(os) = t.stats.as_ref().and_then(|s| s.os.clone()) {
                right.push(sep());
                right.push(small(os, theme::SLATE_300));
            }
        }
        None => left.push(small(tr("세션 없음").into(), theme::TEXT_3)),
    }
    if let Some(u) = &app.update {
        right.push(sep());
        right.push(
            button(
                row![crate::icons::tinted(&ARROW_UP, 11.0, theme::ACCENT_TX), text(tf("{0} 업데이트", &[&u.version])).size(11).color(theme::ACCENT_TX)]
                    .spacing(4)
                    .align_y(Alignment::Center),
            )
                .padding([1, 6])
                .style(btn_style(Btn::Ghost))
                .on_press(Message::UpdateAsk)
                .into(),
        );
    }
    // The sidebar lists the servers while it is open.
    if !app.settings.sidebar_open {
        right.push(sep());
        right.push(small(tf("서버 {0}", &[&app.hosts.len()]), theme::TEXT_3));
    }
    let bar = row![
        row(left).spacing(10).align_y(Alignment::Center),
        Space::new().width(Length::Fill),
        row(right).spacing(12).align_y(Alignment::Center),
    ]
    .align_y(Alignment::Center);
    container(column![rule(true), container(bar).padding([0, 16]).height(STATUSBAR_H - 1.0).center_y(STATUSBAR_H - 1.0)])
        .width(Length::Fill)
        .style(fill(theme::BG_1))
        .into()
}

// ------------------------------------------------------------------ body

fn body(app: &CoShell) -> El<'_> {
    if app.host_view || app.tabs.is_empty() {
        return crate::ui_servers::page(app);
    }
    let Some(tab) = app.active_tab() else { return empty_state() };
    match &tab.phase {
        Phase::Live(s) => {
            let id = tab.id;
            let term: El<'_> =
                TerminalView::new(s.term(), s.size(), theme::term_palette(app.settings.scheme), move |b| Message::Input(id, b), move |g| Message::Resize(id, g))
                    .font(app.term_font, app.settings.font_size)
                    .option_as_meta(app.settings.option_as_meta)
                    .on_link(Message::OpenLink)
                    .active(app.terminal_active(tab))
                    .focus_request(app.term_focus.0, app.term_focus.1)
                    .into();
            // Block view on top while the shell is at a prompt or running a plain command;
            // full-screen programs get the raw terminal.
            if let Some(screen) = app.screen_of(tab) {
                return stack![term, opaque(crate::ui_term::view(app, tab, screen))].into();
            }
            let mut layers = vec![term];
            // No block view (a full-screen program has the terminal): the chat still opens over it.
            if let Some((_, q)) = app.ask.as_ref().filter(|(id, _)| *id == tab.id) {
                layers.push(crate::ui_term::chat_overlay(app, tab, q));
            }
            if tab.exited {
                layers.push(exited_banner(tab));
            }
            if let Some(s) = &app.search {
                layers.push(search_bar(s));
            }
            stack(layers).into()
        }
        Phase::Connecting => dark(center(connect_card(tab))),
        Phase::Failed(e) => dark(center(failed_card(tab, e))),
    }
}

/// Shown over a session that ended: the output stays readable and selectable.
fn exited_banner(tab: &Tab) -> El<'_> {
    let (label, again) = if tab.is_local() { (tr("셸이 종료되었습니다"), tr("새 셸 시작")) } else { (tr("연결이 끊어졌습니다"), tr("다시 연결")) };
    let card = container(
        row![
            text("●").size(9).color(theme::ERR),
            text(label).size(12.5),
            Space::new().width(12),
            btn(tr("탭 닫기"), Btn::Ghost, Some(Message::Close(tab.id))),
            btn(again, Btn::Primary, Some(Message::Retry(tab.id))),
        ]
        .spacing(8)
        .align_y(Alignment::Center),
    )
    .padding([8, 12])
    .style(card_style(alpha(theme::ERR, 0.45)));
    container(card).center_x(Length::Fill).align_bottom(Length::Fill).padding(EXITED_BANNER_BOTTOM).into()
}

/// ⌘F bar at the top right of the terminal.
fn search_bar(s: &Search) -> El<'_> {
    let status: El<'_> = match s.found {
        Some(false) => text(tr("결과 없음")).size(11).color(theme::ERR_TX).into(),
        _ => Space::new().into(),
    };
    let has_query = !s.query.is_empty();
    let card = container(
        row![
            crate::icons::tinted(&SEARCH, 14.0, theme::TEXT_2),
            text_input(tr("터미널 출력 검색"), &s.query)
                .id(SEARCH_INPUT)
                .on_input(Message::SearchQuery)
                .on_submit(Message::SearchNext(false))
                .font(crate::fonts::MONO)
                .size(12)
                .padding([5, 0])
                .width(Length::Fill)
                .style(bare_input_style),
            status,
            icon_btn(&ARROW_UP, tr("이전 결과"), Btn::Ghost, has_query.then_some(Message::SearchNext(false))),
            icon_btn(&ARROW_DOWN, tr("다음 결과"), Btn::Ghost, has_query.then_some(Message::SearchNext(true))),
            icon_btn(&X, tr("닫기"), Btn::Ghost, Some(Message::SearchClose)),
        ]
        .spacing(6)
        .align_y(Alignment::Center),
    )
    .padding(Padding { top: 4.0, right: 4.0, bottom: 4.0, left: 12.0 })
    .max_width(SEARCH_BAR_INPUT_W + 170.0)
    .style(|_| container::Style {
        background: Some(Background::Color(alpha(theme::BG_1, 0.97))),
        border: Border { color: theme::LINE_2, width: 1.0, radius: R_CARD.into() },
        shadow: iced::Shadow { color: Color::from_rgba(0.0, 0.0, 0.0, 0.4), offset: iced::Vector::new(0.0, 8.0), blur_radius: 24.0 },
        ..Default::default()
    });
    container(opaque(card)).width(Length::Fill).align_right(Length::Fill).padding(Padding { top: SEARCH_BAR_TOP, right: 16.0, bottom: 0.0, left: 16.0 }).into()
}

fn close_dialog(tab: &Tab) -> El<'_> {
    let running = tab.tunnels.len();
    let mut col = column![
        text(tf("{0} 연결을 닫을까요?", &[&tab.display_name()])).size(14),
        muted(tr("서버에서 실행 중이던 작업이 중단될 수 있습니다.")),
    ]
    .spacing(6);
    if running > 0 {
        col = col.push(muted(tf("열어 둔 터널 {0}개도 함께 닫힙니다.", &[&running])));
    }
    col = col.push(Space::new().height(8)).push(
        row![
            Space::new().width(Length::Fill),
            btn(tr("취소"), Btn::Ghost, Some(Message::CloseCancel)),
            btn(tr("연결 닫기"), Btn::Danger, Some(Message::CloseConfirmed(tab.id))),
        ]
        .spacing(8),
    );
    container(col).padding(18).width(MODAL_W - 80.0).style(card_style(env_color(tab.env).map_or(theme::LINE_2, |c| alpha(c, 0.6)))).into()
}

/// "Delete this server?", asked from the sidebar's right-click menu.
fn delete_dialog<'a>(app: &CoShell, alias: &'a str) -> El<'a> {
    let col = column![
        text(tf("{0} 서버를 삭제할까요?", &[&app.display_of(alias)])).size(14),
        muted(tr("저장된 접속 정보가 지워지며 되돌릴 수 없습니다.")),
        Space::new().height(8),
        row![
            Space::new().width(Length::Fill),
            btn(tr("취소"), Btn::Ghost, Some(Message::HostDeleteCancel)),
            btn(tr("삭제"), Btn::Danger, Some(Message::HostDeleteConfirmed(alias.to_string()))),
        ]
        .spacing(8),
    ]
    .spacing(6);
    container(col).padding(18).width(MODAL_W - 80.0).style(card_style(env_color(app.env_of(alias)).map_or(theme::LINE_2, |c| alpha(c, 0.6)))).into()
}

pub(crate) fn dark<'a>(content: impl Into<El<'a>>) -> El<'a> {
    container(content).width(Length::Fill).height(Length::Fill).style(|_| container::Style {
        background: Some(Background::Color(theme::BG_0)),
        ..Default::default()
    })
    .into()
}

fn step_status(st: Step) -> (&'static str, Color, String) {
    match st {
        Step::Wait => ("○", theme::TEXT_3, tr("대기").to_string()),
        Step::Connecting => ("◌", theme::ACCENT, tr("연결 중…").into()),
        Step::Authenticating => ("◌", theme::ACCENT, tr("인증 중…").into()),
        Step::Done(d) => ("✓", theme::OK, tf("완료 · {0}ms", &[&d.as_millis()])),
    }
}

/// Plain SSH is the common case: one line. Hops are listed only when there are jump hosts.
fn connect_card(tab: &Tab) -> El<'_> {
    let hops = tab.route.len().saturating_sub(1);
    let mut col = column![text(tf("{0}에 연결 중", &[&tab.name])).size(14)].spacing(6);
    if hops == 0 {
        let (mark, color, status) = step_status(tab.steps.first().copied().unwrap_or(Step::Wait));
        let target = tab.route.first().cloned().unwrap_or_default();
        col = col.push(Space::new().height(4)).push(step_row(mark, color, target, status, color));
    } else {
        col = col.push(muted(tf("경유 서버 {0}곳을 거쳐 연결합니다", &[&hops])));
        col = col.push(Space::new().height(4));
        for (i, label) in tab.route.iter().enumerate() {
            let (mark, color, status) = step_status(tab.steps.get(i).copied().unwrap_or(Step::Wait));
            let name = if i + 1 == tab.route.len() { label.clone() } else { tf("{0}  (경유)", &[&label]) };
            col = col.push(step_row(mark, color, name, status, color));
        }
    }
    col = col.push(Space::new().height(8)).push(row![Space::new().width(Length::Fill), btn(tr("취소"), Btn::Ghost, Some(Message::Close(tab.id)))]);
    container(col).padding(18).width(440).style(card_style(theme::LINE_2)).into()
}

fn step_row<'a>(mark: &'a str, mark_color: Color, name: String, status: String, status_color: Color) -> El<'a> {
    row![
        container(text(mark).size(12).color(mark_color)).width(18),
        mono(name, 12.0, theme::TEXT),
        Space::new().width(Length::Fill),
        text(status).size(11).color(status_color),
    ]
    .spacing(8)
    .align_y(Alignment::Center)
    .into()
}

fn failed_card<'a>(tab: &'a Tab, err: &'a str) -> El<'a> {
    container(
        column![
            text(tf("{0}에 연결하지 못했습니다", &[&tab.name])).size(14).color(theme::ERR_TX),
            text(err.to_string()).size(12).color(theme::TEXT_2),
            Space::new().height(8),
            row![
                Space::new().width(Length::Fill),
                btn(tr("탭 닫기"), Btn::Ghost, Some(Message::Close(tab.id))),
                btn(tr("다시 시도"), Btn::Primary, Some(Message::Retry(tab.id))),
            ]
            .spacing(8),
        ]
        .spacing(6),
    )
    .padding(18)
    .width(MODAL_W)
    .style(card_style(alpha(theme::ERR, 0.45)))
    .into()
}

fn about_dialog(app: &CoShell) -> El<'_> {
    use crate::app::AboutCheck;
    let status: El<'_> = match (&app.about_check, &app.update) {
        (Some(AboutCheck::Checking), _) => text(tr("업데이트 확인 중…")).size(12).color(theme::TEXT_3).into(),
        (_, Some(u)) => button(text(tf("{0} 버전으로 업데이트", &[&u.version])).size(12).color(theme::ACCENT))
            .padding([2, 8])
            .style(btn_style(Btn::Ghost))
            .on_press(Message::UpdateAsk)
            .into(),
        (Some(AboutCheck::Done), None) => text(tr("최신 버전입니다")).size(12).color(theme::TEXT_3).into(),
        (Some(AboutCheck::Failed(e)), None) => text(tf("업데이트를 확인하지 못했습니다: {0}", &[e])).size(12).color(theme::ERR).into(),
        (None, None) => Space::new().into(),
    };
    container(
        column![
            crate::icons::icon(&crate::icons::APP, 80.0),
            text("CoShell").size(20),
            text(tf("버전 {0}", &[&crate::VERSION])).size(12.5).color(theme::TEXT_2),
            muted(tr("SSH 접속, 터널, AI 도우미를 한곳에서 쓰는 터미널")),
            status,
            Space::new().height(6),
            btn(tr("닫기"), Btn::Plain, Some(Message::About)),
        ]
        .spacing(6)
        .align_x(Alignment::Center),
    )
    .padding(24)
    .width(320)
    .style(card_style(theme::LINE_2))
    .into()
}

/// "Update now?": what happens, release notes, later / now.
fn update_dialog(u: &crate::update::Release, busy: bool) -> El<'_> {
    let bundled = crate::update::running_bundle().is_some();
    let what = if bundled {
        tr("내려받아 SHA-256을 확인한 뒤 앱을 바꾸고 다시 시작합니다. 열린 SSH 세션과 터미널은 닫힙니다.")
    } else {
        tr("개발 빌드라 여기서 바로 설치할 수 없습니다. 릴리스 페이지를 엽니다.")
    };
    let now = if busy { tr("내려받는 중…") } else if bundled { tr("지금 업데이트") } else { tr("릴리스 페이지 열기") };
    container(
        column![
            row![crate::icons::icon(&crate::icons::APP, 28.0), text(tf("CoShell {0}", &[&u.version])).size(15)].spacing(10).align_y(Alignment::Center),
            text(tf("지금 쓰는 버전은 {0}입니다.", &[&crate::VERSION])).size(12).color(theme::TEXT_2),
            muted(what),
            Space::new().height(6),
            row![
                btn(tr("릴리스 노트"), Btn::Ghost, (!u.page.is_empty()).then(|| Message::OpenLink(u.page.clone()))),
                Space::new().width(Length::Fill),
                btn(tr("나중에"), Btn::Ghost, (!busy).then_some(Message::UpdateCancel)),
                btn(now, Btn::Primary, (!busy).then_some(Message::UpdateInstall)),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        ]
        .spacing(8),
    )
    .padding(20)
    .width(MODAL_W)
    .style(card_style(theme::LINE_2))
    .into()
}

fn empty_state<'a>() -> El<'a> {
    dark(center(
        column![
            crate::icons::icon(&crate::icons::APP, 72.0),
            text("CoShell").size(20),
            muted(tr("서버에 접속하거나 로컬 셸을 여세요")),
            Space::new().height(12),
            row![btn(tr("서버 열기  ⌘K"), Btn::Plain, Some(Message::OpenPalette)), btn(tr("새 로컬 셸  ⌘T"), Btn::Plain, Some(Message::NewLocal))]
                .spacing(8),
        ]
        .spacing(6)
        .align_x(Alignment::Center),
    ))
}

// ------------------------------------------------------------------ palette

fn palette(app: &CoShell) -> El<'_> {
    let Some(p) = &app.palette else { return Space::new().into() };
    let items = app.palette_items();
    let rows: Vec<El<'_>> = items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let (title, sub) = match item {
                PaletteItem::Host(h) => {
                    let e = &app.hosts[*h];
                    let user = e.user.as_deref().map(|u| format!("{u}@")).unwrap_or_default();
                    let port = e.port.filter(|p| *p != coshell_ssh::DEFAULT_PORT).map(|p| format!(":{p}")).unwrap_or_default();
                    let via = e.proxy_jump.as_deref().map(|j| format!("  ·  via {}", j.replace(',', " › "))).unwrap_or_default();
                    let name = app.display_of(&e.alias);
                    let title = match app.env_of(&e.alias) {
                        Env::None => name,
                        env => format!("{} · {name}", env.label()),
                    };
                    (title, format!("{user}{}{port}{via}", e.host()))
                }
                PaletteItem::Direct(q) => (tf("직접 접속: {0}", &[&q]), tr("~/.ssh/config 규칙 적용").into()),
                PaletteItem::NewLocal => (tr("새 로컬 셸").into(), "⌘T".into()),
                PaletteItem::NewHost => (tr("새 SSH 호스트 추가").into(), tr("연결 설정").into()),
                PaletteItem::Servers => (tr("서버 관리").into(), "⌘O".into()),
                PaletteItem::Settings => (tr("설정").into(), "⌘,".into()),
                PaletteItem::About => (tr("CoShell 정보").into(), tf("버전 {0}", &[&crate::VERSION])),
                PaletteItem::AiCheck => (tr("현재 서버 자동 헬스체크 실행").into(), tr("AI 진단").into()),
                PaletteItem::AiAttach => (tr("터미널 출력을 AI 질문에 첨부").into(), tr("출력 첨부").into()),
                PaletteItem::NewChat => (tr("새 AI 대화 시작").into(), tr("새 대화").into()),
                PaletteItem::Tunnels => (tr("포트 터널 열기").into(), tr("터널").into()),
                PaletteItem::ClearTerm => (tr("현재 터미널 화면 지우기").into(), "clear".into()),
            };
            let kind = match item {
                PaletteItem::Host(_) | PaletteItem::Direct(_) => tr("서버 전환"),
                PaletteItem::AiCheck | PaletteItem::AiAttach | PaletteItem::NewChat => tr("AI 코파일럿"),
                PaletteItem::NewHost | PaletteItem::Servers | PaletteItem::Tunnels => tr("호스트"),
                PaletteItem::NewLocal | PaletteItem::ClearTerm => tr("터미널"),
                PaletteItem::Settings | PaletteItem::About => "CoShell",
            };
            let selected = i == p.selected;
            let tag = container(text(kind).size(10).color(theme::TEXT_2)).padding([2, 8]).style(|_| container::Style {
                background: Some(Background::Color(theme::BG_0)),
                border: Border { color: theme::LINE, width: 1.0, radius: 4.0.into() },
                ..Default::default()
            });
            button(
                row![
                    column![
                        text(title).size(12).font(MEDIUM).color(if selected { theme::ACCENT_HI } else { theme::SLATE_100 }),
                        mono(sub, 11.0, theme::TEXT_3)
                    ]
                    .spacing(2)
                    .width(Length::Fill),
                    tag,
                ]
                .spacing(10)
                .align_y(Alignment::Center),
            )
            .width(Length::Fill)
            .padding([10, 12])
            .on_press(Message::PalettePick(item.clone()))
            .style(move |_, status| button::Style {
                background: Some(Background::Color(
                    if selected || matches!(status, button::Status::Hovered) { theme::BG_3 } else { Color::TRANSPARENT },
                )),
                text_color: theme::TEXT,
                border: Border { radius: R_CARD.into(), ..Default::default() },
                ..Default::default()
            })
            .into()
        })
        .collect();
    let list: El<'_> = if rows.is_empty() {
        container(text(tr("검색 결과가 없습니다.")).size(12).color(theme::TEXT_3)).center_x(Length::Fill).padding([24, 0]).into()
    } else {
        scroll(column(rows).spacing(4).padding(8)).height(Length::Shrink).into()
    };
    let card = container(
        column![
            container(
                row![
                    crate::icons::tinted(&SEARCH, 16.0, theme::ACCENT_TX),
                    text_input(tr("서버 전환, 빠른 명령어 실행, AI 진단..."), &p.query)
                        .id(PALETTE_INPUT)
                        .on_input(Message::PaletteQuery)
                        .on_submit(Message::PaletteSubmit)
                        .size(14)
                        .padding([2, 0])
                        .style(bare_input_style),
                    kbd("ESC"),
                ]
                .spacing(10)
                .align_y(Alignment::Center)
            )
            .padding([12, 16]),
            rule(true),
            container(list).max_height(PALETTE_LIST_H),
        ],
    )
    .width(PALETTE_W)
    .clip(true)
    .style(card_style(theme::LINE_2));
    let scrim = mouse_area(container(Space::new()).width(Length::Fill).height(Length::Fill).style(|_| container::Style {
        background: Some(Background::Color(theme::SCRIM)),
        ..Default::default()
    }))
    .on_press(Message::ClosePalette);
    let top = app.settings.window.height * 0.12;
    stack![scrim, container(card).center_x(Length::Fill).padding(Padding { top, right: 16.0, bottom: 16.0, left: 16.0 })].into()
}

// ------------------------------------------------------------------ dialogs

fn hostkey_dialog<'a>(tab: &'a Tab, q: &'a HostKeyQuery, confirm: &'a str) -> El<'a> {
    let id = tab.id;
    let target = format!("{}:{}", q.host, q.port);
    let hop = tab.route.get(q.hop).cloned().unwrap_or_else(|| target.clone());
    match &q.kind {
        HostKeyKind::Unknown => container(
            column![
                row![crate::icons::icon(&crate::icons::SAFE_GUARD, 18.0), text(tr("처음 접속하는 서버입니다")).size(14)].spacing(8).align_y(Alignment::Center),
                mono(hop, 12.0, theme::TEXT_2),
                muted(tr("서버가 보낸 호스트 키 지문입니다. 서버 관리자에게 받은 지문과 같은지 확인하세요.")),
                fingerprint_box(format!("{}  {}", q.algorithm, q.fingerprint), theme::LINE_2, theme::TEXT),
                muted(tr("저장하면 known_hosts에 추가되고 다음부터는 묻지 않습니다.")),
                Space::new().height(6),
                row![
                    btn(tr("취소"), Btn::Ghost, Some(Message::HostKeyAnswer(id, HostKeyDecision::Reject))),
                    Space::new().width(Length::Fill),
                    btn(tr("이번만 접속"), Btn::Plain, Some(Message::HostKeyAnswer(id, HostKeyDecision::AcceptOnce))),
                    btn(tr("신뢰하고 저장"), Btn::Primary, Some(Message::HostKeyAnswer(id, HostKeyDecision::AcceptAndSave))),
                ]
                .spacing(8),
            ]
            .spacing(8),
        )
        .padding(18)
        .width(MODAL_W)
        .style(card_style(theme::LINE_2))
        .into(),
        HostKeyKind::Changed { line } => {
            let matches = confirm == q.host;
            container(
                column![
                    row![crate::icons::icon(&crate::icons::SAFE_GUARD, 18.0), text(tr("서버 호스트 키가 변경되었습니다")).size(14).color(theme::ERR_TX)]
                        .spacing(8)
                        .align_y(Alignment::Center),
                    container(
                        text(tf("{0} 의 호스트 키가 known_hosts {1}행과 다릅니다. 서버를 재설치했을 수도 있지만, 누군가 중간에서 통신을 가로채고 있을 수도 있습니다.", &[&hop, &line]))
                        .size(12)
                        .color(theme::ERR_TX)
                    )
                    .padding(10)
                    .style(|_| container::Style {
                        background: Some(Background::Color(alpha(theme::ERR, 0.08))),
                        border: Border { color: alpha(theme::ERR, 0.35), width: 1.0, radius: R_CONTROL.into() },
                        ..Default::default()
                    }),
                    text(tr("지금 받은 키")).size(11).color(theme::TEXT_2),
                    fingerprint_box(format!("{}  {}", q.algorithm, q.fingerprint), alpha(theme::ERR, 0.5), theme::ERR_TX),
                    muted(tr("서버 관리자에게 키 변경 여부를 확인하기 전에는 접속하지 마세요.")),
                    text(tf("교체하려면 서버 주소 \"{0}\"을(를) 입력하세요 (기존 파일은 known_hosts.old로 백업)", &[&q.host]))
                        .size(11)
                        .color(theme::TEXT_2),
                    text_input(tr("서버 주소 입력"), confirm)
                        .id(CONFIRM_INPUT)
                        .on_input(move |s| Message::HostKeyConfirm(id, s))
                        .font(crate::fonts::MONO)
                        .size(12)
                        .padding([7, 10])
                        .style(input_style),
                    Space::new().height(6),
                    row![
                        btn(tr("키 교체 후 접속"), Btn::Danger, matches.then_some(Message::HostKeyAnswer(id, HostKeyDecision::ReplaceAndSave))),
                        Space::new().width(Length::Fill),
                        btn(tr("연결 중단"), Btn::Primary, Some(Message::HostKeyAnswer(id, HostKeyDecision::Reject))),
                    ]
                    .spacing(8),
                ]
                .spacing(8),
            )
            .padding(18)
            .width(MODAL_W + 40.0)
            .style(card_style(alpha(theme::ERR, 0.6)))
            .into()
        }
    }
}

fn fingerprint_box<'a>(s: String, line: Color, color: Color) -> El<'a> {
    container(mono(s, 12.0, color))
        .padding([9, 12])
        .width(Length::Fill)
        .style(move |_| container::Style {
            background: Some(Background::Color(theme::BG_0)),
            border: Border { color: line, width: 1.0, radius: R_CONTROL.into() },
            ..Default::default()
        })
        .into()
}

fn interactive_dialog<'a>(tab: &'a Tab, q: &'a InteractiveQuery, answers: &'a [Zeroizing<String>], save: bool) -> El<'a> {
    let id = tab.id;
    let title = if q.name.is_empty() { tr("인증 필요").to_string() } else { q.name.clone() };
    let mut col = column![text(title).size(14), mono(q.label.clone(), 12.0, theme::TEXT_2)].spacing(8);
    if !q.instructions.is_empty() {
        col = col.push(muted(q.instructions.clone()));
    }
    let last = q.prompts.len().saturating_sub(1);
    for (i, p) in q.prompts.iter().enumerate() {
        let value = answers.get(i).map(|z| z.as_str()).unwrap_or("");
        let mut input = text_input("", value)
            .id(prompt_input_id(i))
            .on_input(move |s| Message::PromptInput(id, i, s))
            .secure(!p.echo)
            .font(crate::fonts::MONO)
            .size(13)
            .padding([8, 10])
            .style(input_style);
        if i == last {
            input = input.on_submit(Message::PromptSubmit(id));
        }
        col = col.push(text(p.text.trim().to_string()).size(12).color(theme::TEXT_2)).push(input);
    }
    if q.password {
        col = col.push(
            iced::widget::checkbox(save)
                .label(tr("이 서버 비밀번호 저장 (접속에 성공하면 키체인에 저장)"))
                .on_toggle(move |on| Message::PromptSave(id, on))
                .size(14)
                .text_size(11.5),
        );
    }
    col = col.push(Space::new().height(6)).push(
        row![
            Space::new().width(Length::Fill),
            btn(tr("취소"), Btn::Ghost, Some(Message::PromptCancel(id))),
            btn(tr("확인"), Btn::Primary, Some(Message::PromptSubmit(id))),
        ]
        .spacing(8),
    );
    container(col).padding(18).width(MODAL_W - 80.0).style(card_style(alpha(theme::WARN, 0.45))).into()
}
