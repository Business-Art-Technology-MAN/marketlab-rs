//! Bridge vector-ga dispatch into OTL series evaluation.

use vector_ga::{compute_cpu, GaComputeRequest, GaDataRef, ParamKV, ParamValue};
use vector_ta::utilities::enums::Kernel;

use super::series_eval::SeriesEvalContext;

pub fn ga_compute_f64(
    indicator_id: &str,
    ctx: &SeriesEvalContext<'_>,
    period: usize,
    q: Option<f64>,
) -> Vec<f64> {
    let params = build_params(period, q);
    ga_compute_output(indicator_id, None, ctx, &params)
}

pub fn ga_compute_output(
    indicator_id: &str,
    output_id: Option<&str>,
    ctx: &SeriesEvalContext<'_>,
    params: &[ParamKV<'_>],
) -> Vec<f64> {
    let data = ga_data_ref(indicator_id, ctx);
    let request = GaComputeRequest {
        indicator_id,
        output_id,
        data,
        params,
        kernel: Kernel::Auto,
    };
    match compute_cpu(request) {
        Ok(result) => result.series.f64_values(),
        Err(_) => vec![f64::NAN; ctx.len()],
    }
}

fn build_params(period: usize, q: Option<f64>) -> Vec<ParamKV<'static>> {
    let mut params = vec![ParamKV {
        key: "period",
        value: ParamValue::Int(period as i64),
    }];
    if let Some(q) = q {
        params.push(ParamKV {
            key: "q",
            value: ParamValue::Float(q),
        });
    }
    params
}

fn ga_data_ref<'a>(indicator_id: &str, ctx: &'a SeriesEvalContext<'a>) -> GaDataRef<'a> {
    match indicator_id {
        "wedge_volume" => GaDataRef::ConstituentMatrix {
            columns: if ctx.constituents.is_empty() {
                std::slice::from_ref(&ctx.primary)
            } else {
                ctx.constituents.as_slice()
            },
        },
        "scalar_beta" | "bivector_beta" | "geometric_beta" => GaDataRef::DualSlice {
            asset: ctx.primary,
            market: ctx.series_named("market"),
        },
        "nnls_weights" => GaDataRef::DualWithMatrix {
            market: ctx.series_named("market"),
            columns: if ctx.constituents.is_empty() {
                std::slice::from_ref(&ctx.primary)
            } else {
                ctx.constituents.as_slice()
            },
        },
        _ => GaDataRef::Slice {
            values: ctx.primary,
        },
    }
}

pub fn is_ga_function(name: &str) -> bool {
    matches!(
        name.strip_prefix("ga::").unwrap_or(name),
        "wedge_volume"
            | "scalar_beta"
            | "bivector_beta"
            | "orientation"
            | "displacement"
            | "rolling_quantile"
            | "nnls_weights"
            | "month_end"
    )
}

pub fn ga_multi_outputs(name: &str) -> Option<&'static [&'static str]> {
    match name.strip_prefix("ga::").unwrap_or(name) {
        "geometric_beta" => Some(&["scalar", "bivector"]),
        _ => None,
    }
}
