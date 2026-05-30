//! Stage Composer trait bindings for the USD hierarchy tree-table.

use std::collections::HashSet;

use gpui::*;
use openusd::sdf::Value;
use pulsar_marketlab_ui::workspace::{
    StageComposerPane, StagePrimRow, StageTreeColumnHost, StageTreeColumnLayout,
    StageTreeColumnHandle,
};

use crate::workspace_state::TradingSystemWorkspace;

fn prim_type_class(path: &str, type_name: Option<&str>) -> &'static str {
    if let Some(type_name) = type_name {
        return match type_name {
            "FinancialAsset" => "Asset",
            "OtlOperator" => "OTL Shader",
            "OtlTaUberSignal" => "TA Uber Signal",
            "PortfolioIntegrator" => "Integrator",
            "Scope" => "Scope",
            _ => "Prim",
        };
    }
    if path.starts_with("/assets/") || path.starts_with("/MarketLab/") {
        "Asset"
    } else if path.starts_with("/analytics/") {
        "OTL Shader"
    } else if path.starts_with("/portfolios/") {
        "Integrator"
    } else if path == "/MarketLab" {
        "Scope"
    } else {
        "Prim"
    }
}

fn prim_weight_allocation(
    usd: &pulsar_marketlab_ui::workspace::ManagedUsdStage,
    path: &str,
) -> String {
    let risk_path = format!("{path}.risk_budget");
    if let Some(Value::Float(weight)) = usd.field(&risk_path, "default") {
        return format!("{weight:.2}");
    }
    let cap_path = format!("{path}.inputs:initial_capital");
    if let Some(Value::Float(capital)) = usd.field(&cap_path, "default") {
        return format!("{capital:.0}");
    }
    "—".to_string()
}

fn prim_strategy_version(
    usd: &pulsar_marketlab_ui::workspace::ManagedUsdStage,
    path: &str,
) -> String {
    let id_path = format!("{path}.inputs:id");
    if let Some(Value::String(id)) = usd.field(&id_path, "default") {
        let trimmed = id.trim_matches('"');
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    "v1".to_string()
}

fn child_paths(rows: &[StagePrimRow], parent_path: &str, parent_depth: usize) -> bool {
    rows.iter().any(|row| {
        row.depth == parent_depth + 1
            && row.path.starts_with(&format!("{parent_path}/"))
    })
}

impl StageComposerPane for TradingSystemWorkspace {
    fn stage_ledger_workspace(&self) -> Entity<pulsar_marketlab_ui::workspace::WorkspaceContext> {
        self.workspace_context.clone()
    }

    fn stage_tree_collapsed_paths(&self) -> &HashSet<String> {
        &self.collapsed_tree_paths
    }

    fn toggle_stage_tree_collapsed(&mut self, path: &str, cx: &mut Context<Self>) {
        if self.collapsed_tree_paths.contains(path) {
            self.collapsed_tree_paths.remove(path);
        } else {
            self.collapsed_tree_paths.insert(path.to_string());
        }
        cx.notify();
    }

    fn select_stage_path(&mut self, path: Option<String>, cx: &mut Context<Self>) {
        TradingSystemWorkspace::select_stage_path(self, path, cx);
    }

    fn stage_prim_rows(&self, cx: &App) -> Vec<StagePrimRow> {
        let stage = self.usd_stage.read(cx);
        let usd = self.workspace_context.read(cx).usd_stage();
        let mut rows: Vec<StagePrimRow> = stage
            .stage_prim_rows()
            .unwrap_or_default()
            .into_iter()
            .map(|row| {
                let type_name = stage.prim_type_name(&row.path);
                let type_class = prim_type_class(&row.path, type_name.as_deref()).to_string();
                let weight_allocation = prim_weight_allocation(usd, &row.path);
                let strategy_version = prim_strategy_version(usd, &row.path);
                StagePrimRow {
                    path: row.path.clone(),
                    label: row.label,
                    depth: row.depth,
                    active: self
                        .workspace_context
                        .read(cx)
                        .usd_stage()
                        .prim_active(&row.path),
                    type_class,
                    weight_allocation,
                    strategy_version,
                    has_children: false,
                }
            })
            .collect();

        for index in 0..rows.len() {
            let path = rows[index].path.clone();
            let depth = rows[index].depth;
            rows[index].has_children = child_paths(&rows, &path, depth);
        }

        rows
    }
}

impl StageTreeColumnHost for TradingSystemWorkspace {
    fn stage_tree_columns(&self) -> StageTreeColumnLayout {
        self.stage_tree_columns
    }

    fn set_stage_tree_columns(&mut self, layout: StageTreeColumnLayout) {
        self.stage_tree_columns = layout;
    }

    fn stage_tree_header_bounds(&self) -> Option<Bounds<Pixels>> {
        self.stage_tree_header_bounds
    }

    fn set_stage_tree_header_bounds(&mut self, bounds: Bounds<Pixels>) {
        self.stage_tree_header_bounds = Some(bounds);
    }

    fn active_stage_tree_column_drag(&self) -> Option<(StageTreeColumnHandle, f32)> {
        self.active_stage_tree_column_drag
    }

    fn set_active_stage_tree_column_drag(
        &mut self,
        drag: Option<(StageTreeColumnHandle, f32)>,
    ) {
        self.active_stage_tree_column_drag = drag;
    }
}
