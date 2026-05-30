use super::registry::{apply_canonical_ta_ports, output_port_kind, sync_otl_shader_ports_from_script, ta_uber_from_legacy_indicator, PortWireKind};
use super::*;

fn sample_asset_node(id: usize, prim_path: &str) -> VisualNode {
    VisualNode {
        id,
        name: "SPY".into(),
        node_type: NodeType::asset_adaptor(prim_path),
        grade: NodeGradeType::Scalar,
        portfolio_allocation_id: None,
        dsl_formula: None,
        aov_outputs: Vec::new(),
        asset_source: None,
        x: 0.0,
        y: 0.0,
        collapsed: false,
        inputs: vec![],
        outputs: vec!["Prim Out".into()],
    }
}

fn sample_ta_node(id: usize) -> VisualNode {
    let mut node = VisualNode {
        id,
        name: "RSI".into(),
        node_type: NodeType::ta_uber_signal(ta_uber_from_legacy_indicator("rsi", 14)),
        grade: NodeGradeType::Scalar,
        portfolio_allocation_id: None,
        dsl_formula: None,
        aov_outputs: Vec::new(),
        asset_source: None,
        x: 0.0,
        y: 0.0,
        collapsed: false,
        inputs: Vec::new(),
        outputs: Vec::new(),
    };
    apply_canonical_ta_ports(&mut node);
    node
}

fn sample_otl_shader_node(id: usize) -> VisualNode {
    VisualNode {
        id,
        name: "Formula".into(),
        node_type: NodeType::otl_shader(String::new()),
        grade: NodeGradeType::Scalar,
        portfolio_allocation_id: None,
        dsl_formula: None,
        aov_outputs: vec!["confidence".into()],
        asset_source: None,
        x: 0.0,
        y: 0.0,
        collapsed: false,
        inputs: vec!["source_stream".into()],
        outputs: vec!["signal".into(), "AOV: confidence".into()],
    }
}

fn sample_portfolio_node(id: usize) -> VisualNode {
    VisualNode {
        id,
        name: "Portfolio".into(),
        node_type: NodeType::portfolio(),
        grade: NodeGradeType::Scalar,
        portfolio_allocation_id: None,
        dsl_formula: None,
        aov_outputs: Vec::new(),
        asset_source: None,
        x: 0.0,
        y: 0.0,
        collapsed: false,
        inputs: vec![portfolio_signal_port_label(0)],
        outputs: vec!["NAV Out".into()],
    }
}

#[test]
fn node_type_tiers_are_explicit() {
    let asset = NodeType::asset_adaptor("/assets/SPY");
    let shader = NodeType::otl_shader("close - sma(3)");
    let ta = NodeType::ta_uber_signal_new(TaArchetype::Oscillator);
    let terminal = NodeType::terminal_integrator("vector_ta");

    assert!(asset.is_asset_adaptor());
    assert!(shader.is_otl_shader());
    assert!(ta.is_ta_uber_signal());
    assert!(terminal.is_terminal_integrator());
    assert_eq!(asset.prim_path(), Some("/assets/SPY"));
    assert_eq!(shader.script(), Some("close - sma(3)"));
    assert_eq!(terminal.engine_target(), Some("vector_ta"));
}

#[test]
fn structural_path_wires_into_ta_source_stream() {
    let asset = sample_asset_node(1, "/assets/SPY");
    let ta = sample_ta_node(2);
    assert!(connection_is_valid(&asset, 0, &ta, 0));
}

#[test]
fn ta_ports_remain_fixed_when_algorithm_changes() {
    let mut node = sample_ta_node(1);
    let inputs_before = node.inputs.clone();
    let outputs_before = node.outputs.clone();
    node.set_overlay_algorithm("macd");
    assert_eq!(node.inputs, inputs_before);
    assert_eq!(node.outputs, outputs_before);
}

#[test]
fn asset_to_portfolio_direct_wire_is_buy_and_hold() {
    let asset = sample_asset_node(1, "/assets/SPY");
    let portfolio = sample_portfolio_node(3);
    assert!(connection_is_valid(&asset, 0, &portfolio, 0));
}

#[test]
fn ta_result_output_wires_into_portfolio() {
    let ta = sample_ta_node(2);
    let portfolio = sample_portfolio_node(3);
    assert!(connection_is_valid(&ta, 0, &portfolio, 0));
}

#[test]
fn portfolio_output_wires_into_parent_portfolio() {
    let sub = sample_portfolio_node(2);
    let master = sample_portfolio_node(3);
    assert!(connection_is_valid(&sub, 0, &master, 0));
}

#[test]
fn otl_shader_ports_resync_from_osl_signature() {
    let mut node = sample_otl_shader_node(2);
    let mut connections = vec![NodeConnection {
        from_node_id: 1,
        from_port_idx: 0,
        to_node_id: 2,
        to_port_idx: 3,
    }];
    let script = r#"
        float source,
        int lookback,
        int threshold,
        output float signal
    {
        signal = sma(source, 3);
    }"#;
    let errors = sync_otl_shader_ports_from_script(&mut node, script, &mut connections);
    assert_eq!(node.inputs.len(), 3);
    assert_eq!(node.outputs.first().map(String::as_str), Some("signal"));
    assert!(connections.is_empty());
    assert!(!errors.is_empty());
}

#[test]
fn otl_shader_aov_output_is_typed_as_aov() {
    let shader = sample_otl_shader_node(2);
    assert_eq!(output_port_kind(&shader, 0), Some(PortWireKind::NumericSignal));
    assert_eq!(output_port_kind(&shader, 1), Some(PortWireKind::Aov));
}

#[test]
fn validate_graph_wiring_reports_invalid_connections() {
    let nodes = vec![sample_asset_node(1, "/assets/SPY"), sample_portfolio_node(3)];
    let connections = vec![NodeConnection {
        from_node_id: 1,
        from_port_idx: 0,
        to_node_id: 3,
        to_port_idx: 0,
    }];
    let snapshot = PipelineGraphSnapshot {
        nodes,
        connections,
        execution_order: Vec::new(),
        dag_valid: true,
        wiring_valid: true,
        wiring_errors: Vec::new(),
    };
    let errors = validate_graph_wiring(&snapshot);
    assert!(errors.is_empty());
}

#[test]
fn ta_compute_prefers_dsl_formula_over_uber_compose() {
    let node = VisualNode {
        id: 2,
        name: "Custom".into(),
        node_type: NodeType::ta_uber_signal(ta_uber_from_legacy_indicator("sma", 14)),
        grade: NodeGradeType::Scalar,
        portfolio_allocation_id: None,
        dsl_formula: Some("close - sma(3)".into()),
        aov_outputs: Vec::new(),
        asset_source: None,
        x: 0.0,
        y: 0.0,
        collapsed: false,
        inputs: vec!["source_stream".into()],
        outputs: vec!["result".into()],
    };
    let mut window = MarketSeriesWindow::default();
    for close in [100.0, 102.0, 101.0, 105.0, 104.0] {
        window.push_close_only(close);
    }
    let value = ta_compute_for_node(&node, &window).expect("dsl value");
    assert!((value - 0.666_667).abs() < 0.01);
}

#[test]
fn ta_compute_uses_otl_shader_script_when_dsl_formula_missing() {
    let node = VisualNode {
        id: 2,
        name: "Custom".into(),
        node_type: NodeType::otl_shader("close"),
        grade: NodeGradeType::Scalar,
        portfolio_allocation_id: None,
        dsl_formula: None,
        aov_outputs: Vec::new(),
        asset_source: None,
        x: 0.0,
        y: 0.0,
        collapsed: false,
        inputs: vec!["source_stream".into()],
        outputs: vec!["signal".into()],
    };
    let mut window = MarketSeriesWindow::default();
    window.push_close_only(104.0);
    assert_eq!(ta_compute_for_node(&node, &window), Some(104.0));
}
