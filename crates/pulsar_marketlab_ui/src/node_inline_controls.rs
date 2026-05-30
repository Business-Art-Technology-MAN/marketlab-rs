//! Recessed inline controls for OTL node bodies (dropdowns, number inputs).

use gpui::{
    App, Entity, Focusable, InteractiveElement as _, IntoElement, ParentElement as _, RenderOnce,
    SharedString, Styled, Window, div, px,
};

use gpui_component::button::{Button, ButtonCustomVariant, ButtonVariants as _};
use gpui_component::input::{Input, InputState};
use gpui_component::{IconName, Sizable, Size, h_flex};

use crate::theme;

fn control_bg() -> gpui::Hsla {
    theme::chrome_color(theme::CONTROL_BG)
}

fn control_text() -> gpui::Hsla {
    theme::chrome_color(theme::CONTROL_TEXT)
}

fn control_focus() -> gpui::Hsla {
    theme::chrome_color(theme::CONTROL_FOCUS)
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

fn step_input_value(state: &Entity<InputState>, delta: i32, window: &mut Window, cx: &mut App) {
    state.update(cx, |state, cx| {
        let current = state.value().trim().parse::<i32>().unwrap_or(1);
        let next = (current + delta).max(1);
        state.set_value(next.to_string(), window, cx);
        state.focus(window, cx);
    });
}

fn input_is_focused(state: &Entity<InputState>, window: &Window, cx: &App) -> bool {
    state.read(cx).focus_handle(cx).is_focused(window)
}

/// DCC recessed number field with compact stepper buttons for node lookback inputs.
#[derive(Clone, IntoElement)]
pub struct NodeNumberInput {
    state: Entity<InputState>,
}

impl NodeNumberInput {
    pub fn new(state: &Entity<InputState>) -> Self {
        Self {
            state: state.clone(),
        }
    }
}

impl RenderOnce for NodeNumberInput {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        render_node_number_input_inner(&self.state, window, cx)
    }
}

fn render_node_number_input_inner(
    state: &Entity<InputState>,
    window: &mut Window,
    cx: &mut App,
) -> impl IntoElement {
    let bg = control_bg();
    let text = control_text();
    let border = control_border();
    let focus = control_focus();
    let stepper = stepper_button_variant(cx);
    let focused = input_is_focused(state, window, cx);
    let entity_id = state.entity_id();

    let decrement = state.clone();
    let increment = state.clone();

    h_flex()
        .id(("node-number-input", entity_id))
        .h_6()
        .rounded(px(4.0))
        .bg(bg)
        .border_1()
        .border_color(if focused { focus } else { border })
        .overflow_hidden()
        .child(
            Button::new(("node-number-minus", entity_id))
                .custom(stepper)
                .with_size(Size::Small)
                .icon(IconName::Minus)
                .compact()
                .tab_stop(false)
                .rounded_none()
                .on_click({
                    move |_, window, cx| step_input_value(&decrement, -1, window, cx)
                }),
        )
        .child(
            Input::new(state)
                .appearance(false)
                .with_size(Size::Small)
                .focus_bordered(false)
                .bordered(false)
                .flex_1()
                .h_6()
                .px_2()
                .bg(bg)
                .text_color(text)
                .text_size(px(10.0)),
        )
        .child(
            Button::new(("node-number-plus", entity_id))
                .custom(stepper)
                .with_size(Size::Small)
                .icon(IconName::Plus)
                .compact()
                .tab_stop(false)
                .rounded_none()
                .on_click({
                    move |_, window, cx| step_input_value(&increment, 1, window, cx)
                }),
        )
}
