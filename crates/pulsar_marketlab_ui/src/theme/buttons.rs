//! DCC button variants — toolbar, filter chips, and secondary actions.

use gpui::{prelude::FluentBuilder as _, App, ElementId, SharedString};
use gpui_component::button::{Button, ButtonCustomVariant, ButtonVariants as _};
use gpui_component::Selectable;

use crate::theme;

fn toolbar_bg() -> gpui::Hsla {
    theme::chrome_color(theme::TOOLBAR_BG)
}

fn toolbar_border() -> gpui::Hsla {
    theme::chrome_color(theme::TOOLBAR_BORDER)
}

fn toolbar_hover() -> gpui::Hsla {
    theme::chrome_color(theme::TOOLBAR_HOVER_BG)
}

fn toolbar_text() -> gpui::Hsla {
    theme::chrome_color(theme::TEXT_SECONDARY)
}

/// Menu bar and top chrome triggers.
pub fn toolbar_button_variant(cx: &App) -> ButtonCustomVariant {
    ButtonCustomVariant::new(cx)
        .color(toolbar_bg())
        .foreground(toolbar_text())
        .border(toolbar_border())
        .hover(toolbar_hover())
        .active(theme::chrome_color(theme::CONTROL_HOVER))
}

/// Integrator ledger filter chips and low-profile tags.
pub fn chip_button_variant(cx: &App, active: bool) -> ButtonCustomVariant {
    let bg = if active {
        theme::chrome_color(theme::CHIP_ACTIVE_BG)
    } else {
        theme::chrome_color(theme::CHIP_IDLE_BG)
    };
    let fg = if active {
        theme::chrome_color(theme::TEXT_PRIMARY)
    } else {
        theme::chrome_color(theme::TEXT_SECONDARY)
    };
    ButtonCustomVariant::new(cx)
        .color(bg)
        .foreground(fg)
        .border(theme::chrome_color(theme::CHIP_BORDER))
        .hover(theme::chrome_color(theme::CHIP_HOVER_BG))
        .active(theme::chrome_color(theme::CHIP_ACTIVE_BG))
}

/// Secondary pane actions (export, auxiliary controls).
pub fn secondary_button_variant(cx: &App) -> ButtonCustomVariant {
    ButtonCustomVariant::new(cx)
        .color(theme::chrome_color(theme::CONTROL_BG))
        .foreground(theme::chrome_color(theme::CONTROL_TEXT))
        .border(theme::chrome_color(theme::CONTROL_BORDER))
        .hover(theme::chrome_color(theme::CONTROL_HOVER))
        .active(theme::chrome_color(theme::CONTROL_HOVER))
}

pub fn dcc_toolbar_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    cx: &App,
) -> Button {
    Button::new(id)
        .custom(toolbar_button_variant(cx))
        .compact()
        .label(label)
}

pub fn dcc_chip_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    active: bool,
    cx: &App,
) -> Button {
    Button::new(id)
        .custom(chip_button_variant(cx, active))
        .compact()
        .label(label)
        .when(active, |button| button.selected(true))
}

pub fn dcc_secondary_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    cx: &App,
) -> Button {
    Button::new(id)
        .custom(secondary_button_variant(cx))
        .compact()
        .label(label)
}
