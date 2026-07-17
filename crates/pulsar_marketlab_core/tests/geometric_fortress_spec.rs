//! GA integration tests for OTL compiler + vector-ga operators.

use pulsar_marketlab_core::{
    compile_script, compile_unified_script, eval_series_primary, CompiledSeries, SeriesEvalContext,
};

fn uptrend(len: usize) -> Vec<f64> {
    (0..len).map(|i| 100.0 + i as f64).collect()
}

#[test]
fn ga_orientation_detects_uptrend() {
    let prices = uptrend(250);
    let closure = compile_script("ga::orientation(data, 200)").expect("compile");
    let out = eval_series_primary(&closure, &prices);
    assert_eq!(out.len(), prices.len());
    assert_eq!(out.last().copied(), Some(1.0));
}

#[test]
fn ga_scalar_beta_dual_input_shader() {
    let asset = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
    let market: Vec<f64> = asset.iter().map(|v| v * 2.0).collect();
    let script = r#"
        float asset,
        float market,
        output float beta
    {
        beta = ga::scalar_beta(asset, market, 5);
    }"#;
    let closure = compile_script(script).expect("compile");
    let ctx = SeriesEvalContext {
        primary: &asset,
        named: [("market", market.as_slice())].into_iter().collect(),
        constituents: Vec::new(),
    };
    let out = closure(&ctx);
    assert!(out.last().copied().unwrap_or(0.0).is_finite());
    assert!((out[5] - 0.5).abs() < 1e-6);
}

#[test]
fn ga_geometric_beta_multi_output() {
    let asset = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0];
    let market: Vec<f64> = asset.iter().map(|v| v * 2.0).collect();
    let script = "fn main(data, market) { return ga::geometric_beta(data, market, 5); }";
    let ctx = SeriesEvalContext {
        primary: &asset,
        named: [("market", market.as_slice())].into_iter().collect(),
        constituents: Vec::new(),
    };
    let compiled = compile_unified_script(script).expect("compile");
    let CompiledSeries::Multi(closure, ports) = compiled else {
        panic!("expected multi output");
    };
    assert_eq!(ports, vec!["outputs:scalar", "outputs:bivector"]);
    let channels = closure(&ctx);
    assert_eq!(channels.len(), 2);
    assert!(channels[0][6].is_finite());
    assert!(channels[1][6].is_finite());
    assert!((channels[0][6] - 0.5).abs() < 1e-6);
}
