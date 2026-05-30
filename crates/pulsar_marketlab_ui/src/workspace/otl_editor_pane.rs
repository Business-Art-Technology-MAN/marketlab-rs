//! OTL Script Editor workspace tab with compile console footer.

use gpui::*;
use gpui::prelude::FluentBuilder;
use gpui_component::button::{Button, ButtonCustomVariant, ButtonVariants as _};
use gpui_component::input::InputState;
use gpui_component::scroll::ScrollableElement;
use gpui_component::{Disableable, Sizable};

use crate::otl_code_editor::render_otl_code_editor;
use crate::theme;

/// Right-hand workstation tabs exposed in the primary layout switcher.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WorkspaceTab {
    #[default]
    ParamInspector,
    OtlEditor,
}

impl WorkspaceTab {
    pub fn label(self) -> &'static str {
        match self {
            Self::ParamInspector => "Param Inspector",
            Self::OtlEditor => "OTL Script Editor",
        }
    }
}

pub trait OtlEditorPane: Sized {
    fn active_workspace_tab(&self) -> WorkspaceTab;
    fn set_active_workspace_tab(&mut self, tab: WorkspaceTab, cx: &mut Context<Self>);

    fn otl_editor_has_target(&self, cx: &App) -> bool;
    fn otl_editor_placeholder(&self) -> &'static str {
        "Select an OtlShader Node on the Canvas to view or edit its underlying Open Trading Language source."
    }
    fn otl_editor_source_title(&self, cx: &App) -> String {
        let _ = cx;
        "OTL Source".to_string()
    }
    fn ensure_otl_editor_input(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Entity<InputState>>;
    fn otl_compile_status(&self) -> &str;
    fn otl_compile_inflight(&self) -> bool;
    fn compile_otl_script(&mut self, cx: &mut Context<Self>);
}

const TEXT: u32 = theme::TEXT_PRIMARY;
const TEXT_MUTED: u32 = theme::TEXT_SECONDARY;
const DIVIDER: u32 = theme::GRID_MAJOR;
const CONSOLE_BG: u32 = theme::ROW_BACKPLATE_B;
const OK_GREEN: u32 = 0x22c55e;
const ERR_RED: u32 = 0xf87171;

fn compile_button_variant(cx: &App) -> ButtonCustomVariant {
    ButtonCustomVariant::new(cx)
        .color(theme::chrome_color(theme::CONTROL_BG))
        .foreground(theme::chrome_color(theme::CONTROL_TEXT))
        .border(theme::chrome_color(theme::CONTROL_BORDER))
        .hover(theme::chrome_color(theme::CONTROL_HOVER))
        .active(theme::chrome_color(theme::CONTROL_HOVER))
}

fn render_compile_console(status: &str, inflight: bool) -> impl IntoElement {
    let (color, prefix) = if inflight {
        (rgb(TEXT_MUTED), "…")
    } else if status.starts_with("[ OK") {
        (rgb(OK_GREEN), "")
    } else if status.starts_with("[ ERROR") || status.starts_with("ERROR") {
        (rgb(ERR_RED), "")
    } else {
        (rgb(TEXT_MUTED), "")
    };

    div()
        .flex_shrink_0()
        .min_h(px(56.0))
        .max_h(px(120.0))
        .overflow_y_scrollbar()
        .px_3()
        .py_2()
        .bg(rgb(CONSOLE_BG))
        .border_t_1()
        .border_color(rgb(DIVIDER))
        .text_size(px(10.0))
        .font_family("monospace")
        .text_color(color)
        .child(if inflight {
            "Compiling script on background thread…".to_string()
        } else if prefix.is_empty() {
            status.to_string()
        } else {
            format!("{prefix} {status}")
        })
}

pub fn render_workspace_tab_bar<H: OtlEditorPane + 'static>(
    view: Entity<H>,
    host: &H,
) -> impl IntoElement {
    let active = host.active_workspace_tab();
    div()
        .flex_shrink_0()
        .flex()
        .gap_1()
        .px_2()
        .py_1()
        .border_b_1()
        .border_color(rgb(DIVIDER))
        .children([WorkspaceTab::ParamInspector, WorkspaceTab::OtlEditor].map(|tab| {
            let is_active = active == tab;
            div()
                .id(("workspace-tab", tab as usize))
                .px_2()
                .py_1()
                .rounded_sm()
                .cursor(CursorStyle::PointingHand)
                .when(is_active, |el| {
                    el.bg(rgb(theme::ROW_BACKPLATE_B))
                        .border_1()
                        .border_color(rgb(DIVIDER))
                })
                .when(!is_active, |el| el.hover(|style| style.bg(rgb(0x18181b))))
                .text_size(px(10.0))
                .font_weight(if is_active {
                    FontWeight::SEMIBOLD
                } else {
                    FontWeight::NORMAL
                })
                .font_family("monospace")
                .text_color(if is_active {
                    rgb(TEXT)
                } else {
                    rgb(TEXT_MUTED)
                })
                .child(tab.label())
                .on_mouse_down(
                    MouseButton::Left,
                    {
                        let view = view.clone();
                        move |_: &MouseDownEvent, _, cx| {
                            view.update(cx, |host, cx| {
                                host.set_active_workspace_tab(tab, cx);
                            });
                            cx.stop_propagation();
                        }
                    },
                )
        }))
}

pub fn render_otl_editor<H: OtlEditorPane + 'static>(
    view: Entity<H>,
    host: &mut H,
    window: &mut Window,
    cx: &mut Context<H>,
) -> AnyElement {
    let has_target = host.otl_editor_has_target(cx);
    let status = host.otl_compile_status().to_string();
    let inflight = host.otl_compile_inflight();
    let source_title = host.otl_editor_source_title(cx);

    if !has_target {
        return div()
            .flex_1()
            .min_h_0()
            .p_4()
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .max_w(px(280.0))
                    .text_center()
                    .text_size(px(11.0))
                    .font_family("monospace")
                    .text_color(rgb(TEXT_MUTED))
                    .child(host.otl_editor_placeholder()),
            )
            .into_any_element();
    }

    let Some(input) = host.ensure_otl_editor_input(window, cx) else {
        return div()
            .flex_1()
            .child("OTL editor unavailable.")
            .into_any_element();
    };

    div()
        .id("otl-script-editor-pane")
        .flex_1()
        .min_h_0()
        .min_w_0()
        .flex()
        .flex_col()
        .child(
            div()
                .flex_shrink_0()
                .flex()
                .items_center()
                .justify_between()
                .gap_2()
                .px_3()
                .py_2()
                .bg(rgb(theme::ROW_BACKPLATE_B))
                .border_b_1()
                .border_color(rgb(DIVIDER))
                .child(
                    div()
                        .text_size(px(10.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(TEXT_MUTED))
                        .child(source_title),
                )
                .child(
                    Button::new("compile-otl-script")
                        .custom(compile_button_variant(cx))
                        .label("Compile Script")
                        .small()
                        .disabled(inflight)
                        .on_click({
                            let view = view.clone();
                            move |_, _, cx| {
                                view.update(cx, |host, cx| host.compile_otl_script(cx));
                            }
                        }),
                ),
        )
        .child(
            div()
                .id("otl-code-editor-region")
                .flex_1()
                .min_h_0()
                .overflow_hidden()
                .flex()
                .flex_col()
                .bg(theme::chrome_color(theme::CODE_EDITOR_BG))
                .child(render_otl_code_editor(&input)),
        )
        .child(render_compile_console(&status, inflight))
        .into_any_element()
}
