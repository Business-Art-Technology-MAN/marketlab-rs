//! Request/response types for vector-ga dispatch (vectorTA-compatible shape).

use vector_ta::utilities::enums::Kernel;

use crate::registry::{GaInputKind, GaValueType};

#[derive(Debug, Clone, thiserror::Error, PartialEq)]
pub enum GaComputeError {
    #[error("unknown indicator `{0}`")]
    UnknownIndicator(String),
    #[error("unknown output `{output_id}` for indicator `{indicator_id}`")]
    UnknownOutput { output_id: String, indicator_id: String },
    #[error("input kind mismatch for `{0}`")]
    InputKindMismatch(String),
    #[error("series length mismatch")]
    LengthMismatch,
    #[error("invalid parameter `{0}`")]
    InvalidParam(String),
    #[error("computation failed: {0}")]
    Failed(String),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ParamValue {
    Int(i64),
    Float(f64),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParamKV<'a> {
    pub key: &'a str,
    pub value: ParamValue,
}

#[derive(Clone, Debug, PartialEq)]
pub enum GaSeries {
    F64(Vec<f64>),
    Bool(Vec<bool>),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GaDataRef<'a> {
    Slice { values: &'a [f64] },
    DualSlice { asset: &'a [f64], market: &'a [f64] },
    ConstituentMatrix { columns: &'a [&'a [f64]] },
    DualWithMatrix {
        market: &'a [f64],
        columns: &'a [&'a [f64]],
    },
}

impl<'a> GaDataRef<'a> {
    pub fn len(&self) -> usize {
        match self {
            Self::Slice { values } => values.len(),
            Self::DualSlice { asset, .. } => asset.len(),
            Self::ConstituentMatrix { columns } => columns.first().map(|c| c.len()).unwrap_or(0),
            Self::DualWithMatrix { market, .. } => market.len(),
        }
    }

    pub fn input_kind(&self) -> GaInputKind {
        match self {
            Self::Slice { .. } => GaInputKind::Slice,
            Self::DualSlice { .. } => GaInputKind::DualSlice,
            Self::ConstituentMatrix { .. } => GaInputKind::ConstituentMatrix,
            Self::DualWithMatrix { .. } => GaInputKind::DualWithMatrix,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GaComputeRequest<'a> {
    pub indicator_id: &'a str,
    pub output_id: Option<&'a str>,
    pub data: GaDataRef<'a>,
    pub params: &'a [ParamKV<'a>],
    pub kernel: Kernel,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GaComputeResult {
    pub output_id: String,
    pub label: String,
    pub series: GaSeries,
    pub value_type: GaValueType,
}

impl GaSeries {
    pub fn f64_values(&self) -> Vec<f64> {
        match self {
            Self::F64(values) => values.clone(),
            Self::Bool(values) => values.iter().map(|v| if *v { 1.0 } else { 0.0 }).collect(),
        }
    }
}
