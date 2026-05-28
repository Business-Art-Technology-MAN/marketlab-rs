//! GPUI shell layout.

mod menu_bar;
mod node_canvas;
mod param_inspector;
mod render_viewport;
mod sidebar_inspector;
mod split_layout;
mod stage_composer;
mod timeline_controls;

use gpui::*;

use pulsar_marketlab_ui::workspace::{render_workstation_layout, WorkstationLayoutHost};

use crate::workspace_state::TradingSystemWorkspace;

impl WorkstationLayoutHost for TradingSystemWorkspace {
    fn render_node_canvas(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        self.render_node_graph(cx)
    }
}

impl Render for TradingSystemWorkspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        render_workstation_layout(self, window, cx)
    }
}
