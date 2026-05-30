//! GPUI shell layout.

mod menu_bar;
mod node_canvas;
mod param_inspector;
mod sidebar_inspector;
mod split_layout;
mod stage_composer;

use gpui::*;

use pulsar_marketlab_ui::workspace::{
    render_workstation_layout, GraphEngineInvalidationHost, NodeCanvasPane, WorkstationLayoutHost,
};

use crate::workspace_state::{stage_time_for_bar_index, TradingSystemWorkspace};

impl NodeCanvasPane for TradingSystemWorkspace {
    fn canvas_tabs(&self) -> &[pulsar_marketlab_ui::workspace::CanvasEnvironmentTab] {
        &self.canvas_tabs
    }

    fn active_canvas_tab(&self) -> usize {
        self.active_canvas_tab
    }

    fn set_active_canvas_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.canvas_tabs.len() {
            self.active_canvas_tab = index;
            cx.notify();
        }
    }

    fn open_aggregator_canvas(
        &mut self,
        node_id: usize,
        label: String,
        scope_path: String,
        cx: &mut Context<Self>,
    ) {
        if let Some(existing) = self
            .canvas_tabs
            .iter()
            .position(|tab| tab.scope_node_id == Some(node_id))
        {
            self.active_canvas_tab = existing;
            cx.notify();
            return;
        }
        self.canvas_tabs
            .push(pulsar_marketlab_ui::workspace::CanvasEnvironmentTab::aggregator(
                label, node_id, scope_path,
            ));
        self.active_canvas_tab = self.canvas_tabs.len() - 1;
        cx.notify();
    }

    fn wiring_alert_messages(&self) -> Vec<String> {
        self.pipeline_graph
            .snapshot()
            .wiring_errors
            .iter()
            .map(|error| error.message.clone())
            .collect()
    }

    fn render_canvas_graph(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        self.render_node_graph(cx)
    }

    fn connect_primitives(
        &mut self,
        source_prim_path: &str,
        target_prim_path: &str,
        cx: &mut Context<Self>,
    ) {
        TradingSystemWorkspace::connect_primitives(self, source_prim_path, target_prim_path, cx);
    }

    fn disconnect_primitives(
        &mut self,
        source_prim_path: &str,
        target_prim_path: &str,
        cx: &mut Context<Self>,
    ) {
        TradingSystemWorkspace::disconnect_primitives(self, source_prim_path, target_prim_path, cx);
    }
}

impl WorkstationLayoutHost for TradingSystemWorkspace {}

impl GraphEngineInvalidationHost for TradingSystemWorkspace {
    fn workspace_context(&self) -> &Entity<pulsar_marketlab_ui::workspace::WorkspaceContext> {
        &self.workspace_context
    }

    fn graph_engine_bootstrapping(&self) -> bool {
        self.bootstrapping
    }

    fn graph_engine_asset_vectors(&self) -> std::collections::HashMap<String, Vec<f64>> {
        let mut vectors = std::collections::HashMap::new();
        for node in &self.nodes {
            if !node.node_type.is_asset_adaptor() {
                continue;
            }
            if let Some(bars) = self.asset_ohlc_history.get(&node.id) {
                vectors.insert(node.name.clone(), bars.iter().map(|bar| bar.close).collect());
            }
        }
        vectors
    }

    fn graph_engine_timeline_len(&self) -> usize {
        self.playhead_total_bars.max(
            self.asset_ohlc_history
                .values()
                .map(|bars| bars.len())
                .max()
                .unwrap_or(0),
        )
    }

    fn graph_engine_last_compiled_generation(&self) -> u64 {
        self.graph_engine_last_compiled_generation
    }

    fn set_graph_engine_last_compiled_generation(&mut self, generation: u64) {
        self.graph_engine_last_compiled_generation = generation;
    }

    fn graph_engine_recompile_inflight(&self) -> bool {
        self.graph_engine_recompile_inflight
    }

    fn set_graph_engine_recompile_inflight(&mut self, inflight: bool) {
        self.graph_engine_recompile_inflight = inflight;
    }

    fn graph_engine_recompile_pending(&self) -> bool {
        self.graph_engine_recompile_pending
    }

    fn set_graph_engine_recompile_pending(&mut self, pending: bool) {
        self.graph_engine_recompile_pending = pending;
    }

    fn apply_graph_engine_streams(
        &mut self,
        streams: Vec<pulsar_marketlab_core::ComputedAttributeStream>,
        cx: &mut Context<Self>,
    ) {
        for stream in &streams {
            for (bar_index, value) in &stream.samples {
                let bar_idx = *bar_index as usize;
                let time = self
                    .asset_ohlc_history
                    .values()
                    .find_map(|bars| stage_time_for_bar_index(bars, bar_idx))
                    .unwrap_or(*bar_index);
                let _ = self.market_stage.set_sample(
                    &stream.prim_path,
                    &stream.attribute,
                    time,
                    *value as f32,
                );
            }
        }

        self.synchronize_inspector_view();
        self.invalidate_playhead_evaluation_cache();
        self.spawn_playhead_evaluation_async(cx);
    }
}

impl Render for TradingSystemWorkspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.node_lookback_inputs_ready {
            cx.defer_in(window, |this, window, cx| {
                this.ensure_node_lookback_inputs(window, cx);
            });
        }
        render_workstation_layout(self, window, cx)
    }
}
