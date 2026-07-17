//! vectorTA-style wrapper for geometric algebra indicators.

use vector_ga::{
    compute_cpu, get_indicator, list_indicators, GaComputeRequest, GaDataRef, GaSeries, ParamKV,
    ParamValue,
};
use vector_ta::utilities::enums::Kernel;

/// One output channel from a GA indicator sweep.
#[derive(Clone, Debug, PartialEq)]
pub struct GaSeriesOutput {
    pub output_id: String,
    pub label: String,
    pub values: Vec<f64>,
    pub is_boolean: bool,
}

/// Multi-column window for GA operators (mirrors [`MarketSeriesWindow`] shape).
#[derive(Clone, Debug, Default)]
pub struct GaMultiSeriesWindow {
    pub primary: Vec<f64>,
    pub market: Vec<f64>,
    pub constituents: Vec<Vec<f64>>,
}

impl GaMultiSeriesWindow {
    pub fn len(&self) -> usize {
        self.primary.len()
    }
}

pub fn ga_indicator_catalog() -> Vec<&'static str> {
    list_indicators().iter().map(|info| info.id).collect()
}

pub fn compute_ga_all_outputs(
    indicator_id: &str,
    window: &GaMultiSeriesWindow,
    period: usize,
) -> Option<Vec<GaSeriesOutput>> {
    let info = get_indicator(indicator_id)?;
    let params = [ParamKV {
        key: "period",
        value: ParamValue::Int(period as i64),
    }];
    let column_refs: Vec<&[f64]> = if window.constituents.is_empty() {
        vec![window.primary.as_slice()]
    } else {
        window.constituents.iter().map(|c| c.as_slice()).collect()
    };
    let market = if window.market.is_empty() {
        window.primary.as_slice()
    } else {
        window.market.as_slice()
    };
    let mut outputs = Vec::new();
    let output_infos: Vec<(&str, &str)> = if info.outputs.is_empty() {
        vec![("value", "Value")]
    } else {
        info.outputs
            .iter()
            .map(|o| (o.id, o.label))
            .collect()
    };
    for (output_id, label) in output_infos {
        let data = ga_data_ref(indicator_id, window.primary.as_slice(), market, &column_refs);
        let request = GaComputeRequest {
            indicator_id,
            output_id: Some(output_id),
            data,
            params: &params,
            kernel: Kernel::Auto,
        };
        let result = compute_cpu(request).ok()?;
        outputs.push(GaSeriesOutput {
            output_id: output_id.to_string(),
            label: label.to_string(),
            values: result.series.f64_values(),
            is_boolean: matches!(result.series, GaSeries::Bool(_)),
        });
    }
    Some(outputs)
}

fn ga_data_ref<'a>(
    indicator_id: &str,
    primary: &'a [f64],
    market: &'a [f64],
    columns: &'a [&'a [f64]],
) -> GaDataRef<'a> {
    match indicator_id {
        "wedge_volume" => GaDataRef::ConstituentMatrix { columns },
        "scalar_beta" | "bivector_beta" | "geometric_beta" => GaDataRef::DualSlice {
            asset: primary,
            market,
        },
        "nnls_weights" => GaDataRef::DualWithMatrix {
            market,
            columns,
        },
        _ => GaDataRef::Slice { values: primary },
    }
}
