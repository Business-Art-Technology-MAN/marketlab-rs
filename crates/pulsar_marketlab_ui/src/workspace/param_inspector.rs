//! Param Inspector pane: OTL script editor, AOV toggles, and global pipeline overview.

use gpui::*;
use gpui_component::checkbox::Checkbox;
use gpui_component::input::{Input, InputState};
use gpui_component::scroll::ScrollableElement;

use crate::theme;

/// Pipeline-wide summary shown when no USD prim is selected.
#[derive(Clone, Debug)]
pub struct GlobalPipelineOverview {
    pub edit_target_layer: String,
    pub total_assets: usize,
    pub active_sinks: usize,
    pub compilation_status: String,
    pub graph_revision: u64,
    pub computed_stream_count: usize,
    pub last_compile_ms: u64,
    pub playhead_eval_status: String,
    pub stage_overlay_kib: u64,
}

pub trait ParamInspectorPane: Sized {
    fn param_inspector_title(&self) -> String;
    fn param_inspector_global_overview(&self, _cx: &App) -> Option<GlobalPipelineOverview> {
        None
    }
    fn ensure_otl_script_input(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState>;
    fn otl_editing_enabled(&self) -> bool;
    fn aov_channel_options(&self) -> Vec<(String, bool)>;
    fn toggle_aov_channel(&mut self, channel: &str, enabled: bool, cx: &mut Context<Self>);
    fn render_param_inspector_extensions(
        &mut self,
        cx: &mut Context<Self>,
    ) -> AnyElement;
}

const ROW_A: u32 = theme::ROW_BACKPLATE_A;
const ROW_B: u32 = theme::ROW_BACKPLATE_B;
const DIVIDER: u32 = theme::GRID_MAJOR;
const TEXT: u32 = theme::TEXT_PRIMARY;
const TEXT_MUTED: u32 = theme::TEXT_SECONDARY;

fn overview_row(label: &str, value: impl IntoElement, row_index: usize) -> impl IntoElement {
    let backplate = if row_index % 2 == 0 { ROW_A } else { ROW_B };
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap_2()
        .px_3()
        .py_1p5()
        .bg(rgb(backplate))
        .border_b_1()
        .border_color(rgb(DIVIDER))
        .child(
            div()
                .text_size(px(10.0))
                .text_color(rgb(TEXT_MUTED))
                .child(label.to_string()),
        )
        .child(
            div()
                .text_size(px(10.0))
                .font_family("monospace")
                .text_color(rgb(TEXT))
                .child(value),
        )
}

fn render_global_pipeline_overview(overview: GlobalPipelineOverview) -> impl IntoElement {
    div()
        .flex_col()
        .rounded_md()
        .border_1()
        .border_color(rgb(DIVIDER))
        .overflow_hidden()
        .child(
            div()
                .px_3()
                .py_2()
                .bg(rgb(ROW_B))
                .border_b_1()
                .border_color(rgb(DIVIDER))
                .text_size(px(10.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(TEXT_MUTED))
                .child("Pipeline Overview"),
        )
        .child(overview_row(
            "Edit Target Layer",
            overview.edit_target_layer,
            0,
        ))
        .child(overview_row(
            "Total Assets",
            overview.total_assets.to_string(),
            1,
        ))
        .child(overview_row(
            "Active Sinks",
            overview.active_sinks.to_string(),
            2,
        ))
        .child(overview_row(
            "Compilation Latency",
            overview.compilation_status,
            3,
        ))
        .child(
            div()
                .px_3()
                .py_2()
                .bg(rgb(ROW_B))
                .border_b_1()
                .border_color(rgb(DIVIDER))
                .text_size(px(10.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(TEXT_MUTED))
                .child("Global Performance"),
        )
        .child(overview_row(
            "Graph Revision",
            overview.graph_revision.to_string(),
            4,
        ))
        .child(overview_row(
            "Computed Streams",
            overview.computed_stream_count.to_string(),
            5,
        ))
        .child(overview_row(
            "Last Compile",
            format!("{} ms", overview.last_compile_ms),
            6,
        ))
        .child(overview_row("Playhead Eval", overview.playhead_eval_status, 7))
        .child(overview_row(
            "Stage Overlay",
            format!("{} KiB", overview.stage_overlay_kib),
            8,
        ))
}

pub fn render_param_inspector<H: ParamInspectorPane + 'static>(
    view: Entity<H>,
    host: &mut H,
    window: &mut Window,
    cx: &mut Context<H>,
) -> impl IntoElement {
    let title = host.param_inspector_title();
    let global_overview = host.param_inspector_global_overview(cx);
    let otl_enabled = host.otl_editing_enabled();
    let aov_options = host.aov_channel_options();
    let otl_input = host.ensure_otl_script_input(window, cx);

    let mut body = div()
        .flex_1()
        .min_h_0()
        .overflow_y_scrollbar()
        .p_3()
        .flex_col()
        .gap_3()
        .child(
            div()
                .text_xs()
                .font_weight(FontWeight::BOLD)
                .text_color(rgb(TEXT))
                .child(title),
        );

    if let Some(overview) = global_overview {
        body = body.child(render_global_pipeline_overview(overview));
    }

    if otl_enabled {
        body = body
            .child(
                div()
                    .text_size(px(10.0))
                    .text_color(rgb(TEXT_MUTED))
                    .child("OTL Script"),
            )
            .child(
                Input::new(&otl_input)
                    .h(px(72.0))
                    .text_size(px(11.0))
                    .font_family("monospace"),
            );

        let mut aov_list = div().flex_col().gap_1();
        aov_list = aov_list.child(
            div()
                .text_size(px(10.0))
                .text_color(rgb(TEXT_MUTED))
                .child("AOV Outbound Pins"),
        );
        for (channel_index, (channel, enabled)) in aov_options.into_iter().enumerate() {
            let channel_id = channel.clone();
            aov_list = aov_list.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        Checkbox::new(("aov-channel", channel_index))
                            .checked(enabled)
                            .on_click({
                                let view = view.clone();
                                move |checked, window, cx| {
                                    let channel_id = channel_id.clone();
                                    view.update(cx, |host, cx| {
                                        host.toggle_aov_channel(&channel_id, *checked, cx);
                                    });
                                    let _ = window;
                                }
                            }),
                    )
                    .child(
                        div()
                            .text_xs()
                            .font_family("monospace")
                            .text_color(rgb(0x22d3ee))
                            .child(channel),
                    ),
            );
        }
        body = body.child(aov_list);
    }

    body = body.child(host.render_param_inspector_extensions(cx));

    div().size_full().flex().flex_col().child(body)
}
