//! Integration tests: OSL C-style scripts → `compile_unified_script` → closure execution.

use crate::orchestration::compiler::{eval_series_primary, CompiledSeries, MultiSeriesClosure, SeriesClosure};
use crate::orchestration::script_resolve::compile_unified_script;
use crate::SeriesEvalContext;

/// Mock daily close stream with mild trend and oscillation (deterministic).
fn mock_close_prices(bars: usize) -> Vec<f64> {
    (0..bars)
        .map(|i| {
            let t = i as f64;
            100.0 + t * 0.35 + (t * 0.11).sin() * 2.5 + (t * 0.03).cos() * 0.8
        })
        .collect()
}

/// Assert closure output length and post-warmup finiteness (allows indicator prefix NaNs).
fn assert_series_output_valid(output: &[f64], input_len: usize, warmup_bars: usize) {
    assert_eq!(
        output.len(),
        input_len,
        "output length must match input timeline"
    );
    assert!(
        !output.is_empty(),
        "output must contain at least one sample"
    );
    for (index, value) in output.iter().enumerate().skip(warmup_bars) {
        assert!(
            value.is_finite(),
            "bar {index} must be a finite float, got {value}"
        );
    }
    assert!(
        output[warmup_bars..].iter().any(|value| value.is_finite()),
        "expected at least one finite post-warmup sample"
    );
}

fn assert_multi_output_valid(channels: &[Vec<f64>], input_len: usize, warmup_bars: usize) {
    assert!(
        !channels.is_empty(),
        "multi-series closure must emit at least one channel"
    );
    for (channel_index, channel) in channels.iter().enumerate() {
        assert_series_output_valid(channel, input_len, warmup_bars);
        assert!(
            channel.iter().skip(warmup_bars).any(|value| value.is_finite()),
            "channel {channel_index} must contain finite post-warmup values"
        );
    }
}

fn assert_closure_is_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<SeriesClosure>();
    assert_send_sync::<MultiSeriesClosure>();
}

/// User-facing asymmetric adaptive channel script (scalar params from OSL signature).
const ASYMMETRIC_CHANNEL_OSL: &str = r#"
// Asymmetric Adaptive Channel Trigger
void adaptive_trigger(
    float source_stream,
    int trend_period,
    float vol_multiplier,
    float asymmetry_skew,
    output float upper_band,
    output float baseline,
    output float lower_band
) {
    baseline = ta_ema(source_stream, trend_period);
    float standard_dev = ta_stddev(source_stream, trend_period);
    upper_band = baseline + (standard_dev * vol_multiplier * (1.0 + asymmetry_skew));
    lower_band = baseline - (standard_dev * vol_multiplier * (1.0 - asymmetry_skew));
}
"#;

#[test]
fn compile_unified_osl_void_main_generates_executable_series_closure() {
    assert_closure_is_send_sync();

    let compiled = compile_unified_script(ASYMMETRIC_CHANNEL_OSL).expect("OSL script must compile");
    let CompiledSeries::Single(closure) = compiled else {
        panic!("adaptive channel primary output compiles to a single series closure");
    };

    let closes = mock_close_prices(96);
    let output = eval_series_primary(&closure, &closes);

    assert_series_output_valid(&output, closes.len(), 0);
    assert!(
        output.last().copied().unwrap() > 0.0,
        "EMA baseline should stay in a realistic positive price range"
    );
}

#[test]
fn compile_unified_osl_bare_shader_sma_executes_on_mock_closes() {
    let script = r#"
        float source,
        output float signal
    {
        float standard_dev = ta_stddev(source, 5);
        signal = sma(source, 5);
    }"#;

    let compiled = compile_unified_script(script).expect("bare OSL shader must compile");
    let CompiledSeries::Single(closure) = compiled else {
        panic!("expected single series closure");
    };

    let closes = mock_close_prices(48);
    let output = eval_series_primary(&closure, &closes);

    // SMA(5) leaves four NaN warmup bars.
    assert_series_output_valid(&output, closes.len(), 4);
    assert!(
        output[4].is_finite(),
        "first post-warmup SMA sample must be finite"
    );
}

#[test]
fn compile_unified_osl_multi_channel_bollinger_executes_all_outputs() {
    let compiled = compile_unified_script("ta::bollinger_bands(data, 10, 2.0)")
        .expect("bollinger expression must compile");
    let CompiledSeries::Multi(multi, labels) = compiled else {
        panic!("bollinger compiles to a multi-series closure");
    };

    assert_eq!(labels.len(), 3);
    let closes = mock_close_prices(64);
    let channels = multi(&SeriesEvalContext::primary_only(&closes));
    assert_multi_output_valid(&channels, closes.len(), 9);
}

#[test]
fn compile_unified_osl_void_main_closure_is_deterministic_across_invocations() {
    let compiled = compile_unified_script(ASYMMETRIC_CHANNEL_OSL).expect("compile");
    let CompiledSeries::Single(closure) = compiled else {
        panic!("expected single series");
    };

    let closes = mock_close_prices(32);
    let first = eval_series_primary(&closure, &closes);
    let second = eval_series_primary(&closure, &closes);
    assert_eq!(first, second, "series closure must be deterministic");
}

#[cfg(test)]
mod binary_pipeline_tests {
    use crate::orchestration::binary::{OtcBinaryDecoder, OtcBinaryEncoder};

    #[test]
    fn test_otc_binary_roundtrip_and_version_safety() {
        let mock_manifest_json = r#"{"inputs":[{"name":"source_stream","type":"float"}],"outputs":[{"name":"upper_band","type":"float"}]}"#;
        let mock_bytecode = vec![0xDE, 0xAD, 0xBE, 0xEF];

        let current_generation = 2;
        let serialized_bytes = OtcBinaryEncoder::new()
            .with_engine_generation(current_generation)
            .with_manifest(mock_manifest_json)
            .with_bytecode(&mock_bytecode)
            .encode()
            .expect("Failed to serialize OTL node into binary bytecode");

        assert_eq!(&serialized_bytes[0..4], b"OTCB");

        let decoded_asset = OtcBinaryDecoder::decode(&serialized_bytes)
            .expect("Failed to parse and decode valid OTCB binary stream");

        assert_eq!(decoded_asset.manifest_json, mock_manifest_json);
        assert_eq!(decoded_asset.bytecode, mock_bytecode);
        assert_eq!(decoded_asset.header.engine_generation, current_generation);

        let outdated_local_system_generation = 1;
        let validation_result = decoded_asset
            .header
            .validate_compatibility(outdated_local_system_generation);

        assert!(
            validation_result.is_err(),
            "The asset pipeline should have rejected a newer engine generation block."
        );

        if let Err(err) = validation_result {
            assert!(
                err.to_string().contains("Asset compatibility mismatch"),
                "Error output must clearly report generation mismatch constraints."
            );
        }
    }
}
