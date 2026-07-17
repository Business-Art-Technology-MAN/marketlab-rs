//! CPU dispatch for vector-ga indicators (vectorTA-style).

use rayon::prelude::*;

use crate::operators::{
    bivector_beta_series, displacement_series, geometric_beta_series, log_returns,
    month_end_flag, nnls_weight_series, orientation_series, rolling_quantile_series,
    scalar_beta_series, wedge_volume_series,
};
use crate::registry::get_indicator;
use crate::types::{
    GaComputeError, GaComputeRequest, GaComputeResult, GaDataRef, GaSeries, ParamValue,
};

pub fn compute_cpu(request: GaComputeRequest<'_>) -> Result<GaComputeResult, GaComputeError> {
    let info = get_indicator(request.indicator_id)
        .ok_or_else(|| GaComputeError::UnknownIndicator(request.indicator_id.to_string()))?;
    if request.data.input_kind() != info.input_kind {
        return Err(GaComputeError::InputKindMismatch(request.indicator_id.to_string()));
    }

    let period = param_usize(request.params, "period", info.params.first().map(|p| p.default_int as usize).unwrap_or(60));
    let output_id = request
        .output_id
        .unwrap_or(info.outputs.first().map(|o| o.id).unwrap_or("value"));

    let output_info = info
        .outputs
        .iter()
        .find(|o| o.id == output_id)
        .ok_or_else(|| GaComputeError::UnknownOutput {
            output_id: output_id.to_string(),
            indicator_id: request.indicator_id.to_string(),
        })?;

    let series = match request.indicator_id {
        "wedge_volume" => {
            let GaDataRef::ConstituentMatrix { columns } = request.data else {
                return Err(GaComputeError::InputKindMismatch("wedge_volume".into()));
            };
            let returns: Vec<Vec<f64>> = columns.iter().map(|c| log_returns(c)).collect();
            let refs: Vec<&[f64]> = returns.iter().map(|v| v.as_slice()).collect();
            GaSeries::F64(wedge_volume_series(&refs, period))
        }
        "scalar_beta" => {
            let GaDataRef::DualSlice { asset, market } = request.data else {
                return Err(GaComputeError::InputKindMismatch("scalar_beta".into()));
            };
            GaSeries::F64(scalar_beta_series(asset, market, period))
        }
        "bivector_beta" => {
            let GaDataRef::DualSlice { asset, market } = request.data else {
                return Err(GaComputeError::InputKindMismatch("bivector_beta".into()));
            };
            GaSeries::F64(bivector_beta_series(asset, market, period))
        }
        "geometric_beta" => {
            let GaDataRef::DualSlice { asset, market } = request.data else {
                return Err(GaComputeError::InputKindMismatch("geometric_beta".into()));
            };
            let (scalar, bivector) = geometric_beta_series(asset, market, period);
            if output_id == "scalar" {
                GaSeries::F64(scalar)
            } else {
                GaSeries::F64(bivector)
            }
        }
        "orientation" => {
            let GaDataRef::Slice { values } = request.data else {
                return Err(GaComputeError::InputKindMismatch("orientation".into()));
            };
            GaSeries::Bool(orientation_series(values, period))
        }
        "displacement" => {
            let GaDataRef::Slice { values } = request.data else {
                return Err(GaComputeError::InputKindMismatch("displacement".into()));
            };
            GaSeries::F64(displacement_series(values, period))
        }
        "rolling_quantile" => {
            let GaDataRef::Slice { values } = request.data else {
                return Err(GaComputeError::InputKindMismatch("rolling_quantile".into()));
            };
            let q = param_f64(
                request.params,
                "q",
                info.params.get(1).map(|p| p.default_int as f64 / 100.0).unwrap_or(0.5),
            );
            GaSeries::F64(rolling_quantile_series(values, period, q))
        }
        "nnls_weights" => {
            let GaDataRef::DualWithMatrix { market, columns } = request.data else {
                return Err(GaComputeError::InputKindMismatch("nnls_weights".into()));
            };
            let returns: Vec<Vec<f64>> = columns.iter().map(|c| log_returns(c)).collect();
            let refs: Vec<&[f64]> = returns.iter().map(|v| v.as_slice()).collect();
            let weights = nnls_weight_series(market, &refs, period);
            let col = output_id
                .strip_prefix("weight_")
                .and_then(|s| s.parse::<usize>().ok())
                .unwrap_or(0);
            GaSeries::F64(
                weights
                    .get(col)
                    .cloned()
                    .unwrap_or_else(|| vec![f64::NAN; market.len()]),
            )
        }
        "month_end" => {
            let GaDataRef::Slice { values } = request.data else {
                return Err(GaComputeError::InputKindMismatch("month_end".into()));
            };
            GaSeries::Bool(month_end_flag(values.len()))
        }
        other => return Err(GaComputeError::UnknownIndicator(other.to_string())),
    };

    Ok(GaComputeResult {
        output_id: output_info.id.to_string(),
        label: output_info.label.to_string(),
        series,
        value_type: output_info.value_type,
    })
}

pub fn compute_cpu_batch(requests: &[GaComputeRequest<'_>]) -> Vec<Result<GaComputeResult, GaComputeError>> {
    requests.par_iter().map(|req| compute_cpu(*req)).collect()
}

fn param_usize(params: &[crate::types::ParamKV<'_>], key: &str, default: usize) -> usize {
    params
        .iter()
        .find(|p| p.key == key)
        .and_then(|p| match p.value {
            ParamValue::Int(v) if v > 0 => Some(v as usize),
            ParamValue::Float(v) if v > 0.0 => Some(v as usize),
            _ => None,
        })
        .unwrap_or(default)
        .max(1)
}

fn param_f64(params: &[crate::types::ParamKV<'_>], key: &str, default: f64) -> f64 {
    params
        .iter()
        .find(|p| p.key == key)
        .and_then(|p| match p.value {
            ParamValue::Int(v) => Some(v as f64 / 100.0),
            ParamValue::Float(v) => Some(v),
        })
        .unwrap_or(default)
}
