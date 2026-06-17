//! Workstation spec trait bindings (viewport swap, layer context).

use gpui::*;
use pulsar_marketlab_ui::workspace::{
    WorkstationLayoutHost, WorkstationShelfHost, WorkstationShelfId,
};

use crate::workspace_state::TradingSystemWorkspace;

impl WorkstationLayoutHost for TradingSystemWorkspace {
    fn param_inspector_workspace(&self) -> Entity<pulsar_marketlab_ui::workspace::WorkspaceContext> {
        self.workspace_context.clone()
    }
}

impl WorkstationShelfHost for TradingSystemWorkspace {
    fn workstation_shelf_state(&self) -> &pulsar_marketlab_ui::workspace::WorkstationShelfState {
        &self.workstation_shelves
    }

    fn toggle_workstation_shelf(
        &mut self,
        shelf: WorkstationShelfId,
        cx: &mut Context<Self>,
    ) {
        self.workstation_shelves.toggle(shelf);
        cx.notify();
    }
}
