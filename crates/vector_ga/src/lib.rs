//! Geometric algebra indicators for MarketLab strategies.
//!
//! Mirrors the vectorTA pipeline: registry → [`GaComputeRequest`] → [`compute_cpu`] / [`compute_cpu_batch`].

mod dispatch;
mod operators;
mod registry;
mod types;

pub use dispatch::{compute_cpu, compute_cpu_batch};
pub use registry::{
    get_indicator, list_indicators, GaIndicatorInfo, GaInputKind, GaOutputInfo, GaParamInfo,
    GaValueType,
};
pub use types::{
    GaComputeError, GaComputeRequest, GaComputeResult, GaDataRef, GaSeries, ParamKV, ParamValue,
};
pub use vector_ta::utilities::enums::Kernel;

pub use operators::{
    bivector_beta_series, displacement_series, geometric_beta_series, log_returns,
    month_end_flag, nnls_weight_series, orientation_series, rolling_quantile_series,
    scalar_beta_series, wedge_volume_series,
};
