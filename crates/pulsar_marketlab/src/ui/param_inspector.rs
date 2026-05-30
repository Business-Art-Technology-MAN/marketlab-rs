//! Param Inspector trait bindings: OTL script (generic shaders) vs TaUberSignal hyperparams.

use gpui::*;
use gpui_component::input::{InputEvent, InputState};
use pulsar_marketlab_ui::workspace::{GlobalPipelineOverview, ParamInspectorPane};

use crate::graph_compiler::{
    effective_otl_script, portfolio_wired_source_count, sync_otl_shader_aov_ports,
};
use crate::workspace_state::TradingSystemWorkspace;

const AOV_CHANNELS: &[&str] = &["confidence", "variance", "raw_signal"];

fn layer_display_name(identifier: &str) -> String {
    std::path::Path::new(identifier)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(identifier)
        .to_string()
}

impl TradingSystemWorkspace {
    fn selected_otl_script_text(&self) -> String {
        let Some(node_id) = self.selected_otl_shader_node_id() else {
            return String::new();
        };
        self.nodes
            .iter()
            .find(|node| node.id == node_id)
            .and_then(|node| {
                node.dsl_formula
                    .clone()
                    .or_else(|| effective_otl_script(node).map(str::to_string))
            })
            .unwrap_or_default()
    }

    pub(crate) fn reset_otl_script_input(&mut self) {
        self.otl_script_input = None;
        self.otl_script_node_id = None;
    }

    fn commit_otl_script(&mut self, node_id: usize, script: String, cx: &mut Context<Self>) {
        let trimmed = script.trim();
        if let Some(node) = self
            .nodes
            .iter_mut()
            .find(|node| node.id == node_id && node.node_type.is_otl_shader())
        {
            if trimmed.is_empty() {
                node.dsl_formula = None;
            } else {
                node.dsl_formula = Some(trimmed.to_string());
            }
            self.sync_pipeline_graph(cx);
            self.invalidate_playhead_evaluation_cache();
            self.recompute_playhead_diagnostics();
            cx.notify();
        }
    }

    fn sync_otl_aov_ports(&mut self, node_id: usize) {
        if let Some(node) = self.nodes.iter_mut().find(|node| node.id == node_id) {
            sync_otl_shader_aov_ports(node);
        }
    }
}

impl ParamInspectorPane for TradingSystemWorkspace {
    fn param_inspector_title(&self) -> String {
        match self.selected_node_id {
            None => "Global Parameters".to_string(),
            Some(selected_id) => {
                let node_name = self
                    .nodes
                    .iter()
                    .find(|node| node.id == selected_id)
                    .map(|node| node.name.as_str())
                    .unwrap_or("Unknown Node");
                format!("Inspector // {node_name}")
            }
        }
    }

    fn param_inspector_global_overview(&self, cx: &App) -> Option<GlobalPipelineOverview> {
        let workspace = self.workspace_context.read(cx);
        if workspace.selected_path().is_some() {
            return None;
        }

        let edit_target_layer = workspace
            .edit_target_layer()
            .map(layer_display_name)
            .or_else(|| {
                workspace
                    .layer_identifiers()
                    .first()
                    .map(|id| layer_display_name(id))
            })
            .unwrap_or_else(|| "—".to_string());

        let total_assets = self
            .nodes
            .iter()
            .filter(|node| node.node_type.is_asset_adaptor())
            .count();

        let active_sinks = self
            .nodes
            .iter()
            .filter(|node| {
                node.node_type.is_portfolio()
                    && portfolio_wired_source_count(&self.connections, node.id) > 0
            })
            .count();

        let compilation_status = if self.graph_engine_recompile_inflight {
            "Compiling…".to_string()
        } else if workspace.is_engine_cache_dirty(self.graph_engine_last_compiled_generation) {
            "Pending recompile".to_string()
        } else if self.graph_engine_last_compile_ms > 0 {
            format!("Ready ({} ms)", self.graph_engine_last_compile_ms)
        } else {
            "Ready".to_string()
        };

        let playhead_eval_status = if self.playhead_eval_inflight {
            if self.playhead_eval_pending {
                "Running (queued)".to_string()
            } else {
                "Running".to_string()
            }
        } else if self.playhead_eval_pending {
            "Queued".to_string()
        } else {
            "Idle".to_string()
        };

        Some(GlobalPipelineOverview {
            edit_target_layer,
            total_assets,
            active_sinks,
            compilation_status,
            graph_revision: self.pipeline_graph.revision(),
            computed_stream_count: workspace.computed_streams().len(),
            last_compile_ms: self.graph_engine_last_compile_ms,
            playhead_eval_status,
            stage_overlay_kib: workspace.usd_stage().overlay_memory_kib(),
        })
    }

    fn ensure_otl_script_input(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        let selected_id = self.selected_otl_shader_node_id();
        if self.otl_script_input.is_some() && self.otl_script_node_id == selected_id {
            return self.otl_script_input.clone().expect("otl input present");
        }

        let initial = self.selected_otl_script_text();
        let input = cx.new(|cx| InputState::new(window, cx).multi_line(true));
        input.update(cx, |state, cx| {
            state.set_value(initial, window, cx);
        });

        let tracked_id = selected_id;
        cx.subscribe(&input, move |this, _, event: &InputEvent, cx| {
            if !matches!(event, InputEvent::Change) {
                return;
            }
            let Some(node_id) = this.otl_script_node_id.or(tracked_id) else {
                return;
            };
            let Some(input) = this.otl_script_input.as_ref() else {
                return;
            };
            let text = input.read(cx).value().to_string();
            this.commit_otl_script(node_id, text, cx);
        })
        .detach();

        self.otl_script_input = Some(input.clone());
        self.otl_script_node_id = selected_id;
        input
    }

    fn otl_editing_enabled(&self) -> bool {
        self.selected_otl_shader_node_id().is_some()
    }

    fn aov_channel_options(&self) -> Vec<(String, bool)> {
        let Some(node_id) = self.selected_otl_shader_node_id() else {
            return Vec::new();
        };
        let Some(node) = self.nodes.iter().find(|node| node.id == node_id) else {
            return Vec::new();
        };
        AOV_CHANNELS
            .iter()
            .map(|channel| {
                (
                    (*channel).to_string(),
                    node.aov_outputs.iter().any(|name| name == *channel),
                )
            })
            .collect()
    }

    fn toggle_aov_channel(&mut self, channel: &str, enabled: bool, cx: &mut Context<Self>) {
        let Some(node_id) = self.selected_otl_shader_node_id() else {
            return;
        };
        let Some(node) = self.nodes.iter_mut().find(|node| node.id == node_id) else {
            return;
        };
        if enabled {
            if !node.aov_outputs.iter().any(|name| name == channel) {
                node.aov_outputs.push(channel.to_string());
            }
        } else {
            node.aov_outputs.retain(|name| name != channel);
        }
        self.sync_otl_aov_ports(node_id);
        self.sync_pipeline_graph(cx);
        self.invalidate_playhead_evaluation_cache();
        self.recompute_playhead_diagnostics();
        cx.notify();
    }

    fn render_param_inspector_extensions(
        &mut self,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if self.selected_asset_node().is_some() {
            return self.render_asset_path_config_row(cx).into_any_element();
        }
        if self.selected_technical_analysis_node().is_some() {
            return self.render_ta_uber_inspector(cx).into_any_element();
        }
        if self.selected_portfolio_node().is_some() {
            return self.render_portfolio_analytics_panel().into_any_element();
        }
        div().into_any_element()
    }
}
