//! Topology dopesheet host bindings for `TradingSystemWorkspace`.

use gpui::*;
use pulsar_marketlab_ui::workspace::{
    DopesheetUiState, LogicalTreeNode, TopologyDopesheetHost, WorkspaceContext,
};

use crate::workspace_state::TradingSystemWorkspace;

impl TopologyDopesheetHost for TradingSystemWorkspace {
    fn topology_workspace(&self) -> Entity<WorkspaceContext> {
        self.workspace_context.clone()
    }

    fn topology_dopesheet_ui_state(&self) -> &DopesheetUiState {
        &self.dopesheet_ui_state
    }

    fn topology_dopesheet_vertical_scroll_handle(&self) -> &ScrollHandle {
        &self.dopesheet_ui_state.vertical_scroll_handle
    }

    fn topology_ensure_strategy_tree(&mut self, cx: &mut Context<Self>) {
        let stage_generation = self.workspace_context.read(cx).engine_cache_generation();
        let sweep_generation = self.graph_engine_last_compiled_generation;
        if self.topology_tree_cache_stage_generation == stage_generation
            && self.topology_tree_cache_sweep_generation == sweep_generation
        {
            return;
        }
        self.topology_tree_cache = self
            .workspace_context
            .read(cx)
            .compile_logical_strategy_tree();
        self.topology_tree_cache_stage_generation = stage_generation;
        self.topology_tree_cache_sweep_generation = sweep_generation;
    }

    fn topology_strategy_tree(&self) -> &[LogicalTreeNode] {
        &self.topology_tree_cache
    }

    fn topology_select_prim(&mut self, path: Option<String>, cx: &mut Context<Self>) {
        self.select_stage_path(path, cx);
    }

    fn topology_toggle_node_expansion(&mut self, prim_path: &str, cx: &mut Context<Self>) {
        self.dopesheet_ui_state.toggle_expansion(prim_path);
        cx.notify();
    }

    fn topology_on_prim_active_changed(&mut self, cx: &mut Context<Self>) {
        self.request_graph_engine_sweep(cx);
        cx.notify();
    }

    fn topology_on_layer_state_changed(&mut self, cx: &mut Context<Self>) {
        cx.notify();
    }
}
