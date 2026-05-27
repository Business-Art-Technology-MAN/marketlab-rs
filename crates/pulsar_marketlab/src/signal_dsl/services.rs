//! OSL-inspired abstract market provider interface (OTL `RenderServices` equivalent).

use std::sync::Arc;

use super::vector::Vector;

/// Thread-safe OTL closure evaluated against a provider at continuous time `t`.
pub type OtlClosure = Arc<dyn Fn(&dyn MarketProviderServices, f64) -> Option<Vector> + Send + Sync>;

/// Pure abstract hooks decoupling OTL from concrete stage storage and execution engines.
pub trait MarketProviderServices: Send + Sync {
    /// Pull a causal multi-point slice for `path` in `[start_time, end_time]`.
    fn sample_timeline(&self, path: &str, start_time: f64, end_time: f64) -> Vec<Vector>;

    /// Query a framing/global attribute at playhead time `t`.
    fn get_global_attribute(&self, name: &str, t: f64) -> Option<Vector>;

    /// Route unevaluated closure capabilities to a registered integrator engine.
    fn execute_integrator(
        &self,
        integrator_name: &str,
        inputs: &[OtlClosure],
        t: f64,
    ) -> Option<Vector>;
}
