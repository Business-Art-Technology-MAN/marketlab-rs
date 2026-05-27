use super::registry::{output_port_kind, PortWireKind};
use super::*;

fn sample_asset_node(id: usize, prim_path: &str) -> VisualNode {
    VisualNode {
        id,
        name: "SPY".into(),
        node_type: NodeType::asset_adaptor(prim_path),
        grade: NodeGradeType::Scalar,
        ta_indicator_id: None,
        ta_lookback_period: 14,
        dsl_formula: None,
        aov_outputs: Vec::new(),
        asset_source: None,
        x: 0.0,
        y: 0.0,
        inputs: vec![],
        outputs: vec!["Prim Out".into()],
    }
}

fn sample_shader_node(id: usize) -> VisualNode {
    VisualNode {
        id,
        name: "RSI".into(),
        node_type: NodeType::otl_shader(String::new()),
        grade: NodeGradeType::Scalar,
        ta_indicator_id: Some("rsi".into()),
        ta_lookback_period: 14,
        dsl_formula: None,
        aov_outputs: vec!["confidence".into()],
        asset_source: None,
        x: 0.0,
        y: 0.0,
        inputs: vec!["Timeline In".into(), "Mix In".into()],
        outputs: vec!["Signal Out".into(), "AOV: confidence".into()],
    }
}

fn sample_portfolio_node(id: usize) -> VisualNode {
    VisualNode {
        id,
        name: "Portfolio".into(),
        node_type: NodeType::portfolio(),
        grade: NodeGradeType::Scalar,
        ta_indicator_id: None,
        ta_lookback_period: 14,
        dsl_formula: None,
        aov_outputs: Vec::new(),
        asset_source: None,
        x: 0.0,
        y: 0.0,
        inputs: vec![portfolio_signal_port_label(0)],
        outputs: vec!["NAV Out".into()],
    }
}

#[test]
fn node_type_tiers_are_explicit() {
    let asset = NodeType::asset_adaptor("/assets/SPY");
    let shader = NodeType::otl_shader("close - sma(3)");
    let terminal = NodeType::terminal_integrator("vector_ta");

    assert!(asset.is_asset_adaptor());
    assert!(shader.is_otl_shader());
    assert!(terminal.is_terminal_integrator());
    assert_eq!(asset.prim_path(), Some("/assets/SPY"));
    assert_eq!(shader.script(), Some("close - sma(3)"));
    assert_eq!(terminal.engine_target(), Some("vector_ta"));
}

#[test]
fn structural_path_wires_into_shader_timeline_port() {
    let asset = sample_asset_node(1, "/assets/SPY");
    let shader = sample_shader_node(2);
    assert!(connection_is_valid(&asset, 0, &shader, 0));
}

#[test]
fn structural_path_rejected_on_shader_numeric_port() {
    let asset = sample_asset_node(1, "/assets/SPY");
    let shader = sample_shader_node(2);
    assert!(!connection_is_valid(&asset, 0, &shader, 1));
}

#[test]
fn asset_to_portfolio_direct_wire_is_rejected() {
    let asset = sample_asset_node(1, "/assets/SPY");
    let portfolio = sample_portfolio_node(3);
    assert!(!connection_is_valid(&asset, 0, &portfolio, 0));
}

#[test]
fn shader_numeric_output_wires_into_portfolio() {
    let shader = sample_shader_node(2);
    let portfolio = sample_portfolio_node(3);
    assert!(connection_is_valid(&shader, 0, &portfolio, 0));
}

#[test]
fn shader_aov_output_is_typed_as_aov() {
    let shader = sample_shader_node(2);
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
    assert_eq!(errors.len(), 1);
    assert!(errors[0].message.contains("incompatible wire kinds"));
}

#[test]
fn ta_compute_prefers_dsl_formula_over_indicator_id() {
    let node = VisualNode {
        id: 2,
        name: "Custom".into(),
        node_type: NodeType::otl_shader(String::new()),
        grade: NodeGradeType::Scalar,
        ta_indicator_id: Some("sma".into()),
        ta_lookback_period: 14,
        dsl_formula: Some("close - sma(3)".into()),
        aov_outputs: Vec::new(),
        asset_source: None,
        x: 0.0,
        y: 0.0,
        inputs: vec!["In".into()],
        outputs: vec!["Out".into()],
    };
    let mut window = MarketSeriesWindow::default();
    for close in [100.0, 102.0, 101.0, 105.0, 104.0] {
        window.push_close_only(close);
    }
    let value = ta_compute_for_node(&node, &window).expect("dsl value");
    assert!((value - 0.666_667).abs() < 0.01);
}

#[test]
fn ta_compute_uses_node_type_script_when_dsl_formula_missing() {
    let node = VisualNode {
        id: 2,
        name: "Custom".into(),
        node_type: NodeType::otl_shader("close"),
        grade: NodeGradeType::Scalar,
        ta_indicator_id: Some("sma".into()),
        ta_lookback_period: 14,
        dsl_formula: None,
        aov_outputs: Vec::new(),
        asset_source: None,
        x: 0.0,
        y: 0.0,
        inputs: vec!["In".into()],
        outputs: vec!["Out".into()],
    };
    let mut window = MarketSeriesWindow::default();
    window.push_close_only(104.0);
    assert_eq!(ta_compute_for_node(&node, &window), Some(104.0));
}
