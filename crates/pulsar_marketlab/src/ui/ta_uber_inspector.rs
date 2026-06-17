//! Shared GPUI helpers for TaUberSignal inspector and canvas chrome.

use gpui::*;
use pulsar_marketlab_core::{
    algorithm_display_label, node_display_name, TaArchetype,
    TaUberSignalConfig,
};

use crate::graph_compiler::NodeType;
use crate::workspace_state::TradingSystemWorkspace;

pub(crate) fn ta_archetype_accent(node_type: &NodeType) -> u32 {
    match node_type {
        NodeType::TaUberSignal { config } => config.archetype.accent_rgb(),
        NodeType::OtlShader { .. } => 0x9b87f5,
        NodeType::TerminalIntegrator { .. } => 0x5eead4,
        NodeType::AssetAdaptor { .. } => 0xd4a054,
    }
}

pub(crate) fn ta_header_tint(archetype: TaArchetype) -> u32 {
    match archetype {
        TaArchetype::Trend => 0x1e3a5f,
        TaArchetype::Volatility => 0x431407,
        TaArchetype::Oscillator => 0x2a1f3d,
        TaArchetype::Channel => 0x064e3b,
    }
}

impl TradingSystemWorkspace {
    pub(crate) fn set_ta_algorithm_for_node(
        &mut self,
        node_id: usize,
        algorithm_id: String,
        cx: &mut Context<Self>,
    ) {
        let Some(node) = self
            .nodes
            .iter_mut()
            .find(|node| node.id == node_id && node.node_type.is_ta_uber_signal())
        else {
            return;
        };
        if let Some(config) = node.node_type.ta_uber_config_mut() {
            config.algorithm = algorithm_id;
            config.normalize_algorithm();
            node.name = node_display_name(config);
        }
        self.commit_ta_uber_parameter_change(cx);
    }

    pub(crate) fn set_ta_period_for_node(&mut self, node_id: usize, period: u32, cx: &mut Context<Self>) {
        if let Some(node) = self.nodes.iter_mut().find(|n| n.id == node_id) {
            node.set_overlay_period(period);
        }
        if self.ta_lookback_scrubbing {
            cx.notify();
            return;
        }
        self.commit_ta_uber_parameter_change(cx);
    }

    pub(crate) fn set_ta_multiplier_for_node(
        &mut self,
        node_id: usize,
        multiplier: f32,
        cx: &mut Context<Self>,
    ) {
        if let Some(config) = self
            .nodes
            .iter_mut()
            .find(|n| n.id == node_id)
            .and_then(|n| n.node_type.ta_uber_config_mut())
        {
            config.multiplier = multiplier.max(0.01);
        }
        self.commit_ta_uber_parameter_change(cx);
    }

    pub(crate) fn set_ta_annualization_for_node(
        &mut self,
        node_id: usize,
        annualization: f32,
        cx: &mut Context<Self>,
    ) {
        if let Some(config) = self
            .nodes
            .iter_mut()
            .find(|n| n.id == node_id)
            .and_then(|n| n.node_type.ta_uber_config_mut())
        {
            config.annualization = annualization.max(1.0);
        }
        self.commit_ta_uber_parameter_change(cx);
    }

    pub(crate) fn set_ta_signal_period_for_node(
        &mut self,
        node_id: usize,
        signal_period: u32,
        cx: &mut Context<Self>,
    ) {
        if let Some(config) = self
            .nodes
            .iter_mut()
            .find(|n| n.id == node_id)
            .and_then(|n| n.node_type.ta_uber_config_mut())
        {
            config.signal_period = signal_period.max(1);
        }
        self.commit_ta_uber_parameter_change(cx);
    }
}

pub(crate) fn algorithm_picker_chip(
    node_id: usize,
    algorithm_id: &'static str,
    label: String,
    active: bool,
    accent: u32,
    cx: &mut Context<TradingSystemWorkspace>,
) -> impl IntoElement {
    div()
        .px_2()
        .py_1()
        .rounded_sm()
        .cursor_pointer()
        .bg(if active { rgb(0x2a1f3d) } else { rgb(0x141417) })
        .border_1()
        .border_color(if active { rgb(accent) } else { rgb(0x222227) })
        .text_size(px(9.0))
        .font_weight(if active {
            FontWeight::SEMIBOLD
        } else {
            FontWeight::NORMAL
        })
        .text_color(if active { rgb(0xe9d5ff) } else { rgb(0xa1a1aa) })
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _event: &MouseDownEvent, _window, cx| {
                this.set_ta_algorithm_for_node(node_id, algorithm_id.to_string(), cx);
                cx.stop_propagation();
            }),
        )
        .child(label)
}

pub(crate) fn hyperparam_stepper(
    label: &str,
    value_label: String,
    node_id: usize,
    delta: i32,
    on_adjust: fn(&mut TradingSystemWorkspace, usize, i32, &mut Context<TradingSystemWorkspace>),
    cx: &mut Context<TradingSystemWorkspace>,
) -> impl IntoElement {
    div()
        .flex_col()
        .gap_1()
        .child(
            div()
                .flex()
                .justify_between()
                .items_center()
                .child(
                    div()
                        .text_size(px(9.0))
                        .text_color(rgb(0xa1a1aa))
                        .child(label.to_string()),
                )
                .child(
                    div()
                        .text_size(px(9.0))
                        .font_family("monospace")
                        .text_color(rgb(0xe9d5ff))
                        .child(value_label),
                ),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(step_button("−", node_id, -delta, on_adjust, cx))
                .child(step_button("+", node_id, delta, on_adjust, cx)),
        )
}

fn step_button(
    glyph: &'static str,
    node_id: usize,
    delta: i32,
    on_adjust: fn(&mut TradingSystemWorkspace, usize, i32, &mut Context<TradingSystemWorkspace>),
    cx: &mut Context<TradingSystemWorkspace>,
) -> impl IntoElement {
    div()
        .px_2()
        .py_1()
        .rounded_sm()
        .cursor_pointer()
        .bg(rgb(0x141417))
        .border_1()
        .border_color(rgb(0x222227))
        .text_size(px(10.0))
        .font_family("monospace")
        .text_color(rgb(0xe9d5ff))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _event: &MouseDownEvent, _window, cx| {
                on_adjust(this, node_id, delta, cx);
                cx.stop_propagation();
            }),
        )
        .child(glyph)
}

pub(crate) fn adjust_period(this: &mut TradingSystemWorkspace, node_id: usize, delta: i32, cx: &mut Context<TradingSystemWorkspace>) {
    use pulsar_marketlab::technical_analysis::clamp_ta_lookback;
    let current = this
        .nodes
        .iter()
        .find(|n| n.id == node_id)
        .map(|n| n.overlay_period().unwrap_or(14))
        .unwrap_or(14);
    let next = clamp_ta_lookback(current.saturating_add_signed(delta) as usize) as u32;
    this.set_ta_period_for_node(node_id, next, cx);
}

pub(crate) fn adjust_signal_period(this: &mut TradingSystemWorkspace, node_id: usize, delta: i32, cx: &mut Context<TradingSystemWorkspace>) {
    let current = this
        .nodes
        .iter()
        .find(|n| n.id == node_id)
        .and_then(|n| n.node_type.ta_uber_config())
        .map(|c| c.signal_period)
        .unwrap_or(9);
    let next = current.saturating_add_signed(delta).max(1);
    this.set_ta_signal_period_for_node(node_id, next, cx);
}

pub(crate) fn archetype_summary(config: &TaUberSignalConfig) -> String {
    format!(
        "{} · {}",
        config.archetype.display_name(),
        algorithm_display_label(&config.algorithm)
    )
}
