//! Inline scalar uniform controls for OTL shader nodes on the canvas.

use std::collections::HashSet;

use gpui::*;
use gpui_component::input::{InputEvent, InputState};
use pulsar_marketlab_core::{
    parse_script_scalar_uniforms, set_script_uniform_default, OslParamType,
};

use crate::graph_compiler::{
    effective_otl_script, sync_otl_shader_ports_from_script, NodeType,
};
use crate::workspace_state::TradingSystemWorkspace;

impl TradingSystemWorkspace {
    pub(crate) fn otl_shader_param_input_key(node_id: usize, param_name: &str) -> (usize, String) {
        (node_id, param_name.to_ascii_lowercase())
    }

    fn default_uniform_value(param: &pulsar_marketlab_core::OslParameter) -> f64 {
        param.default_value.unwrap_or_else(|| match param.ty {
            OslParamType::Int => 14.0,
            OslParamType::Float => 1.0,
            OslParamType::String => 0.0,
        })
    }

    fn format_uniform_input_value(param: &pulsar_marketlab_core::OslParameter, value: f64) -> String {
        match param.ty {
            OslParamType::Int => format!("{}", value.round() as i64),
            OslParamType::Float => value.to_string(),
            OslParamType::String => value.to_string(),
        }
    }

    pub(crate) fn ensure_otl_shader_param_inputs(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.prune_stale_otl_shader_param_inputs();
        let nodes = self
            .nodes
            .iter()
            .filter(|node| node.node_type.is_otl_shader())
            .map(|node| {
                (
                    node.id,
                    effective_otl_script(node).unwrap_or("").to_string(),
                )
            })
            .collect::<Vec<_>>();
        for (node_id, script) in nodes {
            for param in parse_script_scalar_uniforms(&script) {
                self.ensure_otl_shader_param_input(node_id, &param, window, cx);
            }
        }
    }

    pub(crate) fn ensure_otl_shader_param_input(
        &mut self,
        node_id: usize,
        param: &pulsar_marketlab_core::OslParameter,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        let key = Self::otl_shader_param_input_key(node_id, &param.name);
        if let Some(existing) = self.otl_shader_param_inputs.get(&key) {
            return existing.clone();
        }

        let initial = Self::format_uniform_input_value(param, Self::default_uniform_value(param));
        let input = cx.new(|cx| InputState::new(window, cx));
        input.update(cx, |state, cx| {
            state.set_value(initial, window, cx);
        });

        let param_name = param.name.clone();
        let param_ty = param.ty;
        cx.subscribe(&input, move |this, _, event: &InputEvent, cx| {
            if !matches!(event, InputEvent::Change) {
                return;
            }
            let Some(input) = this
                .otl_shader_param_inputs
                .get(&Self::otl_shader_param_input_key(node_id, &param_name))
            else {
                return;
            };
            let raw = input.read(cx).value().trim().to_string();
            let parsed = match param_ty {
                OslParamType::Int => raw.parse::<f64>().unwrap_or(14.0),
                OslParamType::Float => raw.parse::<f64>().unwrap_or(1.0),
                OslParamType::String => return,
            };
            let script = this
                .nodes
                .iter()
                .find(|node| node.id == node_id)
                .and_then(|node| effective_otl_script(node).map(str::to_string));
            let Some(script) = script else {
                return;
            };
            let updated = set_script_uniform_default(&script, &param_name, parsed);
            let mut wiring_corrections = Vec::new();
            if let Some(node) = this.nodes.iter_mut().find(|node| node.id == node_id) {
                if let NodeType::OtlShader { script: slot, .. } = &mut node.node_type {
                    *slot = updated.clone();
                }
                node.dsl_formula = Some(updated.clone());
                wiring_corrections =
                    sync_otl_shader_ports_from_script(node, &updated, &mut this.connections);
            }
            for error in wiring_corrections {
                this.push_status_log(format!("OTL port topology: {}", error.message));
            }
            this.sync_pipeline_graph(cx);
            cx.notify();
        })
        .detach();

        self.otl_shader_param_inputs
            .insert(key, input.clone());
        input
    }

    pub(crate) fn prune_stale_otl_shader_param_inputs(&mut self) {
        let valid_keys = self
            .nodes
            .iter()
            .filter(|node| node.node_type.is_otl_shader())
            .flat_map(|node| {
                effective_otl_script(node)
                    .map(parse_script_scalar_uniforms)
                    .unwrap_or_default()
                    .into_iter()
                    .map(|param| Self::otl_shader_param_input_key(node.id, &param.name))
            })
            .collect::<HashSet<_>>();
        self.otl_shader_param_inputs
            .retain(|key, _| valid_keys.contains(key));
    }

    pub(crate) fn reset_otl_shader_param_inputs(&mut self) {
        self.otl_shader_param_inputs.clear();
    }
}
