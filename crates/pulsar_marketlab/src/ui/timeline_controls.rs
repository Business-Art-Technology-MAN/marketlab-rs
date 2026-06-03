//! OHLC playhead scrubbing and transport-adjacent UI widgets.

use gpui::*;

use crate::graph_compiler::{upstream_price_source_node_id_parts, AssetSourceType, NodeType};
use crate::ohlc_chart_pane::{
    OhlcChartPaneConfig, playhead_index_for_mouse_x, render_ohlc_candlestick_pane,
};
use crate::workspace_state::TradingSystemWorkspace;

impl TradingSystemWorkspace {
    pub(crate) fn set_playhead_index(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.playhead_total_bars < 2 {
            return;
        }
        self.playhead_current = index.min(self.playhead_total_bars - 1);
        self.sync_playhead_time_from_index();
        cx.notify();
        self.sync_view_window(cx);
    }

    pub(crate) fn playhead_index_from_position(&self, position: Point<Pixels>) -> Option<usize> {
        let bounds = self.ohlc_chart_bounds?;
        if self.playhead_total_bars < 2 {
            return None;
        }
        let mouse_x: f32 = position.x.into();
        Some(playhead_index_for_mouse_x(
            mouse_x,
            bounds,
            self.playhead_total_bars,
        ))
    }

    pub(crate) fn begin_playhead_scrub(&mut self, event: &MouseDownEvent, cx: &mut Context<Self>) {
        let Some(index) = self.playhead_index_from_position(event.position) else {
            return;
        };
        self.playhead_scrubbing = true;
        self.set_playhead_index(index, cx);
        cx.stop_propagation();
    }

    pub(crate) fn update_playhead_scrub(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        if !self.playhead_scrubbing {
            return;
        }
        let Some(index) = self.playhead_index_from_position(event.position) else {
            return;
        };
        if index != self.playhead_current {
            self.set_playhead_index(index, cx);
        }
    }

    pub(crate) fn end_playhead_scrub(&mut self, cx: &mut Context<Self>) {
        if self.playhead_scrubbing {
            self.playhead_scrubbing = false;
            cx.notify();
        }
    }
    pub(crate) fn render_ohlc_chart_pane(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut config = OhlcChartPaneConfig::default();

        if let Some(node_id) = self.selected_node_id {
            if let Some(node) = self.nodes.iter().find(|node| node.id == node_id) {
                match &node.node_type {
                    NodeType::AssetAdaptor { .. }
                        if matches!(node.asset_source, Some(AssetSourceType::Csv { .. })) =>
                    {
                        config.asset_name = Some(node.name.clone());
                        config.bars = self
                            .asset_ohlc_history
                            .get(&node_id)
                            .cloned()
                            .unwrap_or_default();
                    }
                    NodeType::TaUberSignal { .. } => {
                        if let Some(uber) = node.node_type.ta_uber_config() {
                            config.apply_uber_signal_overlay(uber);
                        }
                        if let Some(asset_id) = upstream_price_source_node_id_parts(
                            node_id,
                            0,
                            &self.nodes,
                            &self.connections,
                        ) {
                            config.asset_name = self
                                .nodes
                                .iter()
                                .find(|node| node.id == asset_id)
                                .map(|node| node.name.clone());
                            config.bars = self
                                .asset_ohlc_history
                                .get(&asset_id)
                                .cloned()
                                .unwrap_or_default();
                        }
                    }
                    NodeType::OtlShader { .. } => {
                        if let Some(asset_id) = upstream_price_source_node_id_parts(
                            node_id,
                            0,
                            &self.nodes,
                            &self.connections,
                        ) {
                            config.asset_name = self
                                .nodes
                                .iter()
                                .find(|node| node.id == asset_id)
                                .map(|node| node.name.clone());
                            config.bars = self
                                .asset_ohlc_history
                                .get(&asset_id)
                                .cloned()
                                .unwrap_or_default();
                        }
                    }
                    _ => {}
                }
            }
        } else if let Some(bars) = self.asset_ohlc_history.values().next() {
            config.bars = bars.clone();
        }

        self.playhead_total_bars = config.bars.len();
        if self.playhead_total_bars == 0 {
            self.playhead_current = 0;
        } else {
            self.playhead_current = self
                .playhead_current
                .min(self.playhead_total_bars - 1);
        }

        let total_bars = self.playhead_total_bars;
        let playhead = if total_bars >= 2 {
            self.playhead_current.min(total_bars - 1)
        } else {
            0
        };
        config.playhead_index = if total_bars >= 2 {
            Some(playhead)
        } else {
            None
        };

        let view = cx.entity().downgrade();
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .cursor(CursorStyle::PointingHand)
            .on_children_prepainted({
                move |bounds: Vec<Bounds<Pixels>>, _window: &mut Window, cx: &mut App| {
                    if let Some(plot_bounds) = bounds.last() {
                        let _ = view.update(cx, |workspace, _cx| {
                            workspace.ohlc_chart_bounds = Some(*plot_bounds);
                        });
                    }
                }
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, _window, cx| {
                    this.begin_playhead_scrub(event, cx);
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _window, cx| {
                this.update_playhead_scrub(event, cx);
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _event: &MouseUpEvent, _window, cx| {
                    this.end_playhead_scrub(cx);
                }),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .child(render_ohlc_candlestick_pane(config)),
            )
    }
    pub(crate) fn render_bottom_metrics_panel(&self) -> impl IntoElement {
        let mut log_lines = div()
            .flex_col()
            .gap_0p5()
            .mt_2()
            .flex_1();
        for line in &self.pipeline_status_log {
            log_lines = log_lines.child(
                div()
                    .text_size(px(9.0))
                    .font_family("monospace")
                    .text_color(rgb(0x71717a))
                    .child(line.clone()),
            );
        }

        div()
            .h(px(144.0))
            .w_full()
            .bg(rgb(0x0c0c0e))
            .border_t_1()
            .border_color(rgb(0x222227))
            .p_4()
            .flex_col()
            .child(
                div()
                    .text_xs()
                    .font_weight(FontWeight::BOLD)
                    .text_color(rgb(0x34d399))
                    .child("📈 Pipeline Status Console"),
            )
            .child(log_lines)
    }
}
