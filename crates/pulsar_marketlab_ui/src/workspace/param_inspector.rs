//! Param Inspector pane: OTL script editor, AOV toggles, and global pipeline overview.

use gpui::*;
use gpui_component::checkbox::Checkbox;
use gpui_component::input::{Input, InputState};
use gpui_component::scroll::ScrollableElement;
use pulsar_marketlab_core::ComposedAssetMeta;

use super::workstation_shelf::{render_collapsible_shelf, WorkstationShelfHost, WorkstationShelfId};
use crate::otl_inspector::dcc_otl_script_input;
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
    pub view_window_status: String,
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

    /// Commit the focused OTL script field (blur / Cmd+S); default no-op.
    fn commit_focused_otl_script(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {}

    /// When true, show an editable display-name field bound to `info:user_label`.
    fn node_label_editing_enabled(&self) -> bool {
        false
    }

    fn ensure_node_label_input(
        &mut self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Entity<InputState>> {
        None
    }

    fn commit_node_label(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {}
}

const ROW_A: u32 = theme::ROW_BACKPLATE_A;
const ROW_B: u32 = theme::ROW_BACKPLATE_B;
const DIVIDER: u32 = theme::GRID_MAJOR;
const TEXT: u32 = theme::TEXT_PRIMARY;
const TEXT_MUTED: u32 = theme::TEXT_SECONDARY;

fn metadata_display(value: &str) -> String {
    if value.trim().is_empty() {
        "—".to_string()
    } else {
        value.to_string()
    }
}

/// FinanceDatabase `info:*` labels rendered beneath primary asset attribute rows.
pub fn render_composed_asset_metadata_grid(meta: &ComposedAssetMeta) -> impl IntoElement {
    div()
        .mt_2()
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
                .child("Catalog Metadata"),
        )
        .child(overview_row("User Label", metadata_display(&meta.user_label), 0))
        .child(overview_row("Sector", metadata_display(&meta.sector), 1))
        .child(overview_row("Industry", metadata_display(&meta.industry), 2))
        .child(overview_row(
            "Market Cap Class",
            metadata_display(&meta.market_cap_class),
            3,
        ))
        .child(overview_row("Currency", metadata_display(&meta.currency), 4))
        .child(overview_row("Country", metadata_display(&meta.country), 5))
}

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

fn render_pipeline_overview_body(overview: GlobalPipelineOverview) -> impl IntoElement {
    div()
        .flex_col()
        .overflow_hidden()
        .child(overview_row(
            "Edit Target Layer",
            overview.edit_target_layer.clone(),
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
            overview.compilation_status.clone(),
            3,
        ))
}

fn render_global_performance_body(overview: GlobalPipelineOverview) -> impl IntoElement {
    div()
        .flex_col()
        .overflow_hidden()
        .child(overview_row(
            "Graph Revision",
            overview.graph_revision.to_string(),
            0,
        ))
        .child(overview_row(
            "Computed Streams",
            overview.computed_stream_count.to_string(),
            1,
        ))
        .child(overview_row(
            "Last Compile",
            format!("{} ms", overview.last_compile_ms),
            2,
        ))
        .child(overview_row(
            "View Window",
            overview.view_window_status.clone(),
            3,
        ))
        .child(overview_row(
            "Stage Overlay",
            format!("{} KiB", overview.stage_overlay_kib),
            4,
        ))
}

fn render_global_pipeline_overview<H: WorkstationShelfHost + 'static>(
    view: Entity<H>,
    host: &H,
    overview: GlobalPipelineOverview,
) -> impl IntoElement {
    div()
        .flex_col()
        .gap_1()
        .child(render_collapsible_shelf(
            view.clone(),
            host.workstation_shelf_state(),
            WorkstationShelfId::InspectorPipelineOverview,
            "Pipeline Overview",
            false,
            render_pipeline_overview_body(overview.clone()),
        ))
        .child(render_collapsible_shelf(
            view,
            host.workstation_shelf_state(),
            WorkstationShelfId::InspectorGlobalPerformance,
            "Global Performance",
            false,
            render_global_performance_body(overview),
        ))
}

pub fn render_param_inspector<H: ParamInspectorPane + WorkstationShelfHost + 'static>(
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
    let node_label_enabled = host.node_label_editing_enabled();
    let node_label_input = if node_label_enabled {
        host.ensure_node_label_input(window, cx)
    } else {
        None
    };

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

    if let Some(label_input) = node_label_input.as_ref() {
        body = body.child(
            div()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_size(px(10.0))
                        .text_color(rgb(TEXT_MUTED))
                        .child("Display Name"),
                )
                .child(
                    div()
                        .w_full()
                        .rounded_md()
                        .bg(rgb(theme::CONTROL_BG))
                        .border_1()
                        .border_color(rgb(theme::CONTROL_BORDER))
                        .overflow_hidden()
                        .child(
                            Input::new(label_input)
                                .appearance(false)
                                .bordered(false)
                                .focus_bordered(false)
                                .w_full()
                                .h(px(28.0))
                                .text_size(px(11.0))
                                .font_family("monospace")
                                .text_color(rgb(theme::CONTROL_TEXT)),
                        ),
                ),
        );
    }

    if let Some(overview) = global_overview {
        body = body.child(render_global_pipeline_overview(view.clone(), host, overview));
    }

    if otl_enabled {
        body = body
            .child(
                div()
                    .text_size(px(10.0))
                    .text_color(rgb(TEXT_MUTED))
                    .child("OTL Script"),
            )
            .child({
                let view = view.clone();
                dcc_otl_script_input(&otl_input, cx, move |_window, cx| {
                    view.update(cx, |host, cx| host.commit_focused_otl_script(_window, cx));
                })
            });

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
                            .text_color(rgb(theme::SOCKET_AOV))
                            .child(channel),
                    ),
            );
        }
        body = body.child(aov_list);
    }

    body = body.child(host.render_param_inspector_extensions(cx));

    div().size_full().flex().flex_col().child(body)
}
