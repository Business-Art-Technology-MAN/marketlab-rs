//! OTL Phase 2 codegen + vector sweep integration (100-bar window).

use std::collections::HashMap;
use std::sync::Arc;

use pulsar_marketlab_core::{
    compile_object_program, compile_object_tier, evaluate_compiled_tier, ExecutionContext,
    GraphSeriesMatrix, ObjectCodegenRegistry, OtlObjectKind,
};

const BAR_COUNT: usize = 100;
const INITIAL_CAPITAL: f64 = 1_000_000.0;

fn synthetic_close_series() -> Vec<f64> {
    (0..BAR_COUNT)
        .map(|bar| 100.0 + (bar as f64) * 0.25 + ((bar as f64) * 0.1).sin() * 2.0)
        .collect()
}

fn build_ctx(prices: Vec<f64>) -> ExecutionContext {
    let mut asset_vectors = HashMap::new();
    asset_vectors.insert("SPY".to_string(), Arc::from(prices.into_boxed_slice()));
    ExecutionContext::new(
        BAR_COUNT,
        INITIAL_CAPITAL,
        "Allocation::EqualWeight",
        HashMap::new(),
        asset_vectors,
    )
}

#[test]
fn matrix_clear_signal_column_zeros_partial_tier_writes() {
    let mut matrix = GraphSeriesMatrix::with_capacity(8, 1, 1);
    matrix.write_signal(0, 3, 0.75);
    matrix.clear_signal_column(0);
    assert_eq!(matrix.read_signal(0, 3), Some(0.0));
}

#[test]
fn alpha_conviction_signal_allocator_portfolio_sweep_100_bars() {
    let prices = synthetic_close_series();
    let mut ctx = build_ctx(prices.clone());
    let mut matrix = GraphSeriesMatrix::with_capacity(BAR_COUNT, 2, 2);

    let signal_source = r#"
signal alpha_conviction(
    input closure raw,
    output closure gated
) {
    gated = sma(data, 5);
}
"#;
    let signal_program = compile_object_program(signal_source).expect("parse signal OTL");
    assert_eq!(
        signal_program.primary_object().map(|object| object.kind),
        Some(OtlObjectKind::Signal)
    );

    let mut signal_registry = ObjectCodegenRegistry {
        upstream_series: vec![prices.clone()],
        initial_capital: INITIAL_CAPITAL,
        ..ObjectCodegenRegistry::default()
    };
    let mut signal_tier =
        compile_object_tier(&signal_program, &signal_registry).expect("compile signal tier");
    ctx.signal_upstream = prices.clone();
    ctx.signal_output_column = 0;
    evaluate_compiled_tier(&mut ctx, &mut signal_tier, &mut matrix).expect("signal sweep");

    let late_conviction = matrix.read_signal(0, BAR_COUNT - 1).unwrap_or(0.0);
    assert!(
        late_conviction.abs() > f64::EPSILON,
        "expected non-zero alpha conviction on bar {}",
        BAR_COUNT - 1
    );

    let allocator_source = r#"
allocator equal_blend(
    input closure[] legs,
    output closure blended
) {
    blended = mix(legs[0], legs[0], 0.5);
}
"#;
    let allocator_program =
        compile_object_program(allocator_source).expect("parse allocator OTL");
    let mut allocator_registry = ObjectCodegenRegistry {
        allocation_method: "Allocation::EqualWeight".to_string(),
        initial_capital: INITIAL_CAPITAL,
        ..ObjectCodegenRegistry::default()
    };
    allocator_registry.register_signal_column("legs", 0);
    let mut allocator_tier =
        compile_object_tier(&allocator_program, &allocator_registry).expect("compile allocator");
    evaluate_compiled_tier(&mut ctx, &mut allocator_tier, &mut matrix).expect("allocator sweep");

    let portfolio_source = r#"
portfolio master_book(
    input closure[] books,
    output closure execution_map
) {
    execution_map = portfolio_info("execution_map");
}
"#;
    let portfolio_program =
        compile_object_program(portfolio_source).expect("parse portfolio OTL");
    let portfolio_registry = ObjectCodegenRegistry {
        initial_capital: INITIAL_CAPITAL,
        ..ObjectCodegenRegistry::default()
    };
    let mut portfolio_tier =
        compile_object_tier(&portfolio_program, &portfolio_registry).expect("compile portfolio");
    evaluate_compiled_tier(&mut ctx, &mut portfolio_tier, &mut matrix).expect("portfolio sweep");

    let cash_start = matrix.cash_at(0);
    let cash_end = matrix.cash_at(BAR_COUNT - 1);
    assert!(
        (cash_end - cash_start).abs() > 1.0,
        "cash position should move when alpha drives allocation (start={cash_start}, end={cash_end})"
    );
}
