//! OTL Param Inspector key context: Cmd/Ctrl+S commits script without global save.

use gpui::{
    actions, App, Entity, Focusable as _, InteractiveElement as _, IntoElement, KeyBinding,
    ParentElement as _, Styled, Window,
};
use gpui_component::input::{Input, InputState};

use crate::theme;

actions!(otl_inspector, [CommitOtlScript]);

pub const OTL_INSPECTOR_CONTEXT: &str = "OtlInspectorContext";

/// Register OTL inspector save shortcut (call once from application init).
pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-s", CommitOtlScript, Some(OTL_INSPECTOR_CONTEXT)),
        KeyBinding::new("ctrl-s", CommitOtlScript, Some(OTL_INSPECTOR_CONTEXT)),
    ]);
}

/// Recessed OTL script field with focused Cmd/Ctrl+S → commit (does not propagate to global save).
pub fn dcc_otl_script_input(
    input: &Entity<InputState>,
    cx: &App,
    on_commit: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let focus = input.read(cx).focus_handle(cx);
    gpui::div()
        .w_full()
        .min_h(gpui::px(72.0))
        .rounded(gpui::px(4.0))
        .bg(theme::chrome_color(theme::CONTROL_BG))
        .border_1()
        .border_color(theme::chrome_color(theme::CONTROL_BORDER))
        .overflow_hidden()
        .key_context(OTL_INSPECTOR_CONTEXT)
        .track_focus(&focus)
        .on_action(move |_: &CommitOtlScript, window, cx| {
            on_commit(window, cx);
            cx.stop_propagation();
        })
        .child(
            Input::new(input)
                .appearance(false)
                .bordered(false)
                .focus_bordered(false)
                .w_full()
                .h(gpui::px(72.0))
                .text_size(gpui::px(11.0))
                .font_family("monospace")
                .text_color(theme::chrome_color(theme::CONTROL_TEXT)),
        )
}
