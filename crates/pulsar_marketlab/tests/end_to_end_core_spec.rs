//! End-to-end OTL core spec: stage → provider → compiled closure → terminal vector.

use pulsar_marketlab::signal_dsl::{compile_formula, invoke_closure, CompileContext, OtlClosure};
use pulsar_marketlab::stage_bridge::ProductionStageProvider;
use pulsar_marketlab::trading_stage::{asset_prim_path, MarketStage};

fn seed_spy_close_stage(stage: &mut MarketStage) {
    let prim = asset_prim_path("SPY").expect("valid prim");
    for (time, price) in [
        (0.0, 100.0),
        (1.0, 102.0),
        (2.0, 101.0),
        (3.0, 105.0),
        (4.0, 104.0),
    ] {
        stage
            .set_sample(&prim, "close", time, price as f32)
            .expect("seed close");
    }
}

#[test]
fn otl_script_executes_via_production_stage_provider_at_playhead() {
    let mut stage = MarketStage::new();
    seed_spy_close_stage(&mut stage);

    let close_path = "/assets/SPY/close";
    let provider = ProductionStageProvider::new(&stage, close_path);
    let ctx = CompileContext {
        timeline_close_path: close_path.to_string(),
    };

    let closure = compile_formula("close - sma(3)", &ctx).expect("compile otl");
    let playhead = 4.0;
    let value = invoke_closure(&closure, &provider, playhead).expect("execute closure");

    assert!(
        (value - 0.666_667).abs() < 0.01,
        "expected close - sma(3) ≈ 0.667, got {value}"
    );
}

#[test]
fn otl_rsi_integrator_routes_through_production_provider() {
    let mut stage = MarketStage::new();
    seed_spy_close_stage(&mut stage);

    let close_path = "/assets/SPY/close";
    let provider = ProductionStageProvider::new(&stage, close_path);
    let ctx = CompileContext {
        timeline_close_path: close_path.to_string(),
    };

    let closure = compile_formula("rsi(3)", &ctx).expect("compile rsi");
    let value = invoke_closure(&closure, &provider, 4.0).expect("execute rsi");
    assert!(value.is_finite());
    assert!(value >= 0.0 && value <= 100.0);
}

#[test]
fn compiled_closure_and_provider_are_send_sync() {
    let mut stage = MarketStage::new();
    seed_spy_close_stage(&mut stage);
    let provider = ProductionStageProvider::new(&stage, "/assets/SPY/close");
    let closure = compile_formula("close", &CompileContext::default()).expect("compile");

    fn assert_send_sync<T: Send + Sync>(_: &T) {}
    assert_send_sync(&provider);
    assert_send_sync(&closure);

    let _: OtlClosure = closure;
}
