//! Recessed inline controls for OTL node bodies (dropdowns, number inputs).

use gpui::{
    App, Entity, Focusable, InteractiveElement as _, IntoElement, ParentElement as _, RenderOnce,
    SharedString, Styled, Window, div, px,
};

use gpui_component::button::{Button, ButtonCustomVariant, ButtonVariants as _};
use gpui_component::input::{Input, InputState};
use gpui_component::{Sizable, Size, h_flex};

use crate::theme;

fn control_bg() -> gpui::Hsla {
    theme::chrome_color(theme::CONTROL_BG)
}

fn control_text() -> gpui::Hsla {
    theme::chrome_color(theme::CONTROL_TEXT)
}

fn control_caret() -> gpui::Hsla {
    theme::chrome_color(theme::CONTROL_CARET)
}

fn control_border() -> gpui::Hsla {
    theme::chrome_color(theme::CONTROL_BORDER)
}

fn control_hover() -> gpui::Hsla {
    theme::chrome_color(theme::CONTROL_HOVER)
}

fn control_focus() -> gpui::Hsla {
    theme::chrome_color(theme::CONTROL_FOCUS)
}

fn recessed_button_variant(cx: &App) -> ButtonCustomVariant {
    ButtonCustomVariant::new(cx)
        .color(control_bg())
        .foreground(control_text())
        .border(control_border())
        .hover(control_hover())
        .active(control_hover())
}

fn stepper_button_variant(cx: &App) -> ButtonCustomVariant {
    ButtonCustomVariant::new(cx)
        .color(control_bg())
        .foreground(control_text())
        .border(control_bg())
        .hover(control_hover())
        .active(control_hover())
}

/// DCC recessed dropdown trigger for node-body parameter menus.
pub fn node_dropdown_trigger(
    id: impl Into<gpui::ElementId>,
    label: impl Into<SharedString>,
    cx: &App,
) -> Button {
    let fg = control_text();
    let label = label.into();

    Button::new(id)
        .custom(recessed_button_variant(cx))
        .small()
        .w_full()
        .child(
            h_flex()
                .w_full()
                .h_6()
                .px_2()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(px(10.0))
                        .text_color(fg)
                        .truncate()
                        .child(label),
                )
                .child(
                    div()
                        .flex_none()
                        .pl_1()
                        .text_size(px(8.0))
                        .text_color(control_caret())
                        .child("▼"),
                ),
        )
}

fn step_input_value(
    state: &Entity<InputState>,
    delta: f64,
    integer: bool,
    window: &mut Window,
    cx: &mut App,
) {
    state.update(cx, |state, cx| {
        let current = state.value().trim().parse::<f64>().unwrap_or(1.0);
        let next = if integer {
            (current + delta).round().max(1.0)
        } else {
            (current + delta).max(0.0)
        };
        let text = if integer {
            format!("{}", next as i64)
        } else if (next.fract()).abs() < f64::EPSILON {
            format!("{next:.0}")
        } else {
            format!("{next:.4}")
                .trim_end_matches('0')
                .trim_end_matches('.')
                .to_string()
        };
        state.set_value(text, window, cx);
        state.focus(window, cx);
    });
}

fn input_is_focused(state: &Entity<InputState>, window: &Window, cx: &App) -> bool {
    state.read(cx).focus_handle(cx).is_focused(window)
}

/// DCC recessed number field with compact stepper buttons for node scalar uniforms.
#[derive(Clone, IntoElement)]
pub struct NodeNumberInput {
    state: Entity<InputState>,
    integer: bool,
}

impl NodeNumberInput {
    pub fn new(state: &Entity<InputState>) -> Self {
        Self {
            state: state.clone(),
            integer: true,
        }
    }

    pub fn integer(mut self, integer: bool) -> Self {
        self.integer = integer;
        self
    }
}

impl RenderOnce for NodeNumberInput {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = self.state;
        let integer = self.integer;
        let step = if integer { 1.0 } else { 0.1 };
        let entity_id = state.entity_id();
        let decrement = state.clone();
        let increment = state.clone();

        let bg = control_bg();
        let text = control_text();
        let border = control_border();
        let focus = control_focus();
        let stepper = stepper_button_variant(cx);
        let focused = input_is_focused(&state, window, cx);

        h_flex()
            .id(("node-number-input", entity_id))
            .w_full()
            .min_w(px(108.0))
            .h_6()
            .rounded(px(4.0))
            .bg(bg)
            .border_1()
            .border_color(if focused { focus } else { border })
            .overflow_hidden()
            .child(
                Button::new(("node-number-minus", entity_id))
                    .custom(stepper.clone())
                    .with_size(Size::Small)
                    .compact()
                    .tab_stop(false)
                    .rounded_none()
                    .label("−")
                    .on_click({
                        move |_, window, cx| {
                            step_input_value(&decrement, -step, integer, window, cx);
                        }
                    }),
            )
            .child(
                Input::new(&state)
                    .appearance(false)
                    .with_size(Size::Small)
                    .bordered(false)
                    .focus_bordered(false)
                    .rounded_none()
                    .flex_1()
                    .min_w(px(36.0))
                    .h_6()
                    .px_1()
                    .bg(bg)
                    .text_color(text)
                    .text_size(px(10.0))
                    .text_align(gpui::TextAlign::Center),
            )
            .child(
                Button::new(("node-number-plus", entity_id))
                    .custom(stepper)
                    .with_size(Size::Small)
                    .compact()
                    .tab_stop(false)
                    .rounded_none()
                    .label("+")
                    .on_click({
                        move |_, window, cx| {
                            step_input_value(&increment, step, integer, window, cx);
                        }
                    }),
            )
    }
}
