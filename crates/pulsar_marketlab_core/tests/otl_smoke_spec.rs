//! Compile-check the OTL smoke-test example script.

use pulsar_marketlab_core::{canonicalize_otl_source, compile_script, eval_series_primary};

fn read_otl(relative: &str) -> String {
    std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join(relative),
    )
    .unwrap_or_else(|err| panic!("read {relative}: {err}"))
}

#[test]
fn sma_smoke_otl_compiles_and_feed_preview_is_sane() {
    let src = read_otl("examples/otl_smoke_test/sma_smoke.otl").replace("\r\n", "\n");
    let canonical = canonicalize_otl_source(&src).expect("canonicalize");
    assert!(canonical.contains("shader sma_smoke"));

    let closure = compile_script(&canonical).expect("compile");
    let data: Vec<f64> = (0..12).map(|i| 100.0 + f64::from(i)).collect();
    let out = eval_series_primary(&closure, &data);
    assert_eq!(out.len(), data.len());

    // SMA(3) at bar 11 on ramp 100..111 → (109+110+111)/3 = 110
    let last = *out.last().expect("last bar");
    assert!(
        (last - 110.0).abs() < 1e-6,
        "expected SMA(3) last bar ≈ 110.0, got {last}"
    );
}
