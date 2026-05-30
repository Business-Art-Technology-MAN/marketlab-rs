//! OTL Script Editor tab: selection binding, USD sync, and background compilation.

use gpui::*;
use gpui_component::input::InputState;
use openusd::sdf::Value;
use pulsar_marketlab_core::{compile_script, display_name_for_script, parse_script_entry_point_name};
use pulsar_marketlab_ui::new_otl_code_editor_state;
use pulsar_marketlab_ui::OTL_CODE_EDITOR_LANGUAGE;
use pulsar_marketlab_ui::workspace::{OtlEditorPane, WorkspaceTab};

use crate::graph_compiler::{
    effective_otl_script, sync_otl_shader_ports_from_script, NodeType,
};
use crate::workspace_state::TradingSystemWorkspace;

#[derive(Clone, Debug, PartialEq, Eq)]
enum OtlEditorTarget {
    Node(usize),
    StagePrim(String),
}

impl TradingSystemWorkspace {
    fn otl_editor_target(&self, cx: &App) -> Option<OtlEditorTarget> {
        if let Some(node_id) = self.selected_otl_shader_node_id() {
            return Some(OtlEditorTarget::Node(node_id));
        }
        let workspace = self.workspace_context.read(cx);
        let path = workspace.selected_path()?;
        let prim_type = workspace.usd_stage().prim_type_name(path)?;
        if prim_type == "OtlOperator" {
            return Some(OtlEditorTarget::StagePrim(path.to_string()));
        }
        None
    }

    fn otl_editor_script_text(&self, cx: &App) -> String {
        match self.otl_editor_target(cx) {
            Some(OtlEditorTarget::Node(node_id)) => self
                .nodes
                .iter()
                .find(|node| node.id == node_id)
                .and_then(|node| {
                    node.dsl_formula
                        .clone()
                        .or_else(|| effective_otl_script(node).map(str::to_string))
                })
                .unwrap_or_default(),
            Some(OtlEditorTarget::StagePrim(path)) => self
                .workspace_context
                .read(cx)
                .usd_stage()
                .field_string(&path, "inputs:script_src")
                .unwrap_or_default(),
            None => String::new(),
        }
    }

    pub(crate) fn reset_otl_editor_input(&mut self) {
        self.otl_editor_input = None;
        self.otl_editor_binding = None;
    }

    fn otl_editor_binding_key(&self, cx: &App) -> Option<String> {
        let lang = OTL_CODE_EDITOR_LANGUAGE;
        match self.otl_editor_target(cx)? {
            OtlEditorTarget::Node(node_id) => Some(format!("node:{node_id}:{lang}")),
            OtlEditorTarget::StagePrim(path) => Some(format!("prim:{path}:{lang}")),
        }
    }

    fn apply_compiled_otl_script(
        &mut self,
        script: String,
        elapsed_ms: u64,
        cx: &mut Context<Self>,
    ) {
        match self.otl_editor_target(cx) {
            Some(OtlEditorTarget::Node(node_id)) => {
                let mut wiring_corrections = Vec::new();
                if let Some(node) = self
                    .nodes
                    .iter_mut()
                    .find(|node| node.id == node_id && node.node_type.is_otl_shader())
                {
                    let trimmed = script.trim();
                    if trimmed.is_empty() {
                        node.dsl_formula = None;
                    } else {
                        node.dsl_formula = Some(trimmed.to_string());
                    }
                    if let NodeType::OtlShader { script: slot, .. } = &mut node.node_type {
                        *slot = trimmed.to_string();
                    }
                    node.name = display_name_for_script(trimmed, &node.name);
                    wiring_corrections =
                        sync_otl_shader_ports_from_script(node, trimmed, &mut self.connections);
                }
                self.otl_shader_param_inputs
                    .retain(|(node_key, _), _| *node_key != node_id);
                for error in &wiring_corrections {
                    self.push_status_log(format!(
                        "OTL port topology: {}",
                        error.message
                    ));
                }
                self.sync_pipeline_graph(cx);
                self.invalidate_playhead_evaluation_cache();
                self.recompute_playhead_diagnostics();
            }
            Some(OtlEditorTarget::StagePrim(path)) => {
                let workspace_context = self.workspace_context.clone();
                let trimmed = script.trim().to_string();
                workspace_context.update(cx, |context, cx| {
                    context.modify_attribute(
                        &path,
                        "inputs:script_src",
                        Value::String(trimmed),
                        cx,
                    );
                });
            }
            None => {}
        }

        self.otl_compile_status = format!(
            "[ OK: Compiled Series Closure ] {elapsed_ms} ms"
        );
        self.otl_compile_inflight = false;

        let view = cx.entity().downgrade();
        cx.defer(move |cx| {
            let _ = view.update(cx, |this, cx| {
                this.publish_canvas_to_usd_stage(cx);
            });
        });
        cx.notify();
    }
}

impl OtlEditorPane for TradingSystemWorkspace {
    fn active_workspace_tab(&self) -> WorkspaceTab {
        self.active_workspace_tab
    }

    fn set_active_workspace_tab(&mut self, tab: WorkspaceTab, cx: &mut Context<Self>) {
        if self.active_workspace_tab == tab {
            return;
        }
        self.active_workspace_tab = tab;
        if tab == WorkspaceTab::OtlEditor {
            self.reset_otl_editor_input();
        }
        cx.notify();
    }

    fn otl_editor_has_target(&self, cx: &App) -> bool {
        self.otl_editor_target(cx).is_some()
    }

    fn otl_editor_source_title(&self, cx: &App) -> String {
        let script = self.otl_editor_script_text(cx);
        parse_script_entry_point_name(&script)
            .map(|name| format!("OTL Source · {name}"))
            .unwrap_or_else(|| "OTL Source".to_string())
    }

    fn ensure_otl_editor_input(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Entity<InputState>> {
        let binding = self.otl_editor_binding_key(cx)?;
        if self.otl_editor_input.is_some() && self.otl_editor_binding.as_deref() == Some(&binding) {
            return self.otl_editor_input.clone();
        }

        let initial = self.otl_editor_script_text(cx);
        let input = new_otl_code_editor_state(window, cx);
        input.update(cx, |state, cx| {
            state.set_value(initial, window, cx);
        });

        self.otl_editor_input = Some(input.clone());
        self.otl_editor_binding = Some(binding);
        Some(input)
    }

    fn otl_compile_status(&self) -> &str {
        &self.otl_compile_status
    }

    fn otl_compile_inflight(&self) -> bool {
        self.otl_compile_inflight
    }

    fn compile_otl_script(&mut self, cx: &mut Context<Self>) {
        if self.otl_compile_inflight {
            return;
        }
        let Some(_target) = self.otl_editor_target(cx) else {
            self.otl_compile_status =
                "Select an OtlShader node before compiling.".to_string();
            cx.notify();
            return;
        };

        let script = self
            .otl_editor_input
            .as_ref()
            .map(|input| input.read(cx).value().to_string())
            .unwrap_or_else(|| self.otl_editor_script_text(cx));

        if script.trim().is_empty() {
            self.otl_compile_status = "[ ERROR ] Script buffer is empty.".to_string();
            cx.notify();
            return;
        }

        self.otl_compile_inflight = true;
        self.otl_compile_status = "Compiling…".to_string();
        cx.notify();

        let script_for_compile = script.clone();
        let view = cx.entity().downgrade();
        cx.spawn(async move |_, cx| {
            let started = std::time::Instant::now();
            let result = cx
                .background_executor()
                .spawn(async move { compile_script(&script_for_compile) })
                .await;
            let elapsed_ms = started.elapsed().as_millis() as u64;

            let _ = cx.update(|cx| {
                if let Some(view) = view.upgrade() {
                    view.update(cx, |workspace, cx| {
                        match result {
                            Ok(_) => {
                                workspace.apply_compiled_otl_script(script, elapsed_ms, cx);
                            }
                            Err(error) => {
                                workspace.otl_compile_inflight = false;
                                workspace.otl_compile_status =
                                    format!("[ ERROR ] {error}");
                                cx.notify();
                            }
                        }
                    });
                }
            });
        })
        .detach();
    }
}

impl TradingSystemWorkspace {
    pub(crate) fn selected_otl_shader_node_id(&self) -> Option<usize> {
        let selected_id = self.selected_node_id?;
        self.nodes
            .iter()
            .find(|node| node.id == selected_id && node.node_type.is_otl_shader())
            .map(|node| node.id)
    }
}
