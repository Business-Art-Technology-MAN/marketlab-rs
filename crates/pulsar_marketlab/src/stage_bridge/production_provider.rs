//! Split-plane adapter: OpenUSD structural plane + [`MarketStage`] temporal plane.

use std::path::Path;
use std::sync::Arc;

use openusd::sdf::schema::FieldKey;

use crate::execution_engine::{EXECUTION_CASH_ATTR, EXECUTION_CASH_PATH};
use crate::signal_dsl::{MarketProviderServices, OtlClosure, Vector};
use crate::signal_dsl::financial::{execute_financial_integrator, execute_stdlib_integrator};
use crate::technical_analysis::{compute_ta_latest_with_params, MarketSeriesWindow};
use crate::trading_stage::MarketStage;

use super::parse_stage_attribute_path;
use super::usd_spike::UsdStageBridge;

/// Routes OTL evaluation across OpenUSD structure and [`MarketStage`] time series.
#[derive(Clone, Debug)]
pub struct ProductionStageProvider {
    usd_stage: Arc<UsdStageBridge>,
    temporal_stage: Arc<MarketStage>,
    active_path: String,
}

impl ProductionStageProvider {
    pub fn new(
        usd_stage: Arc<UsdStageBridge>,
        temporal_stage: Arc<MarketStage>,
        active_path: impl Into<String>,
    ) -> Self {
        Self {
            usd_stage,
            temporal_stage,
            active_path: active_path.into(),
        }
    }

    pub fn from_usd_root(
        usd_root_path: impl AsRef<Path>,
        temporal_stage: Arc<MarketStage>,
        active_path: impl Into<String>,
    ) -> std::io::Result<Self> {
        let usd_stage = Arc::new(UsdStageBridge::open(usd_root_path)?);
        Ok(Self::new(usd_stage, temporal_stage, active_path))
    }

    pub fn usd_stage(&self) -> &Arc<UsdStageBridge> {
        &self.usd_stage
    }

    pub fn temporal_stage(&self) -> &MarketStage {
        &self.temporal_stage
    }

    pub fn active_path(&self) -> &str {
        &self.active_path
    }

    fn active_prim_path(&self) -> Option<&str> {
        parse_stage_attribute_path(&self.active_path).map(|(prim, _)| prim)
    }

    fn prim_active(&self, prim_path: &str) -> bool {
        self.usd_stage.prim_active(prim_path)
    }

    fn resolve_scalar_attribute(&self, full_path: &str, t: f64) -> Option<Vector> {
        let (prim_path, attribute) = parse_stage_attribute_path(full_path)?;
        if !self.prim_active(prim_path) {
            return None;
        }
        self.temporal_stage
            .resolve_attribute_at(prim_path, attribute, t)
            .map(f64::from)
            .filter(|value| value.is_finite())
            .map(Vector::scalar)
    }

    fn resolve_global_metadata(&self, name: &str) -> Option<Vector> {
        let field = name.strip_prefix("global::")?;
        let prim_path = self.active_prim_path()?;
        let property_path = format!("{prim_path}.{field}");
        self.usd_stage
            .field_f32(&property_path, FieldKey::Default)
            .map(f64::from)
            .map(Vector::scalar)
    }

    fn build_close_window(&self, t: f64, bar_count: usize) -> Option<MarketSeriesWindow> {
        let (prim_path, _) = parse_stage_attribute_path(&self.active_path)?;
        if !self.prim_active(prim_path) || bar_count == 0 {
            return None;
        }
        let start = t - bar_count as f64 + 1.0;
        let samples = self
            .temporal_stage
            .samples_in_time_range(prim_path, "close", start, t);
        if samples.is_empty() {
            return None;
        }
        let mut window = MarketSeriesWindow::default();
        for (time, close) in samples {
            let close = f64::from(close);
            let open = self
                .temporal_stage
                .resolve_attribute_at(prim_path, "open", time)
                .map(f64::from)
                .unwrap_or(close);
            let high = self
                .temporal_stage
                .resolve_attribute_at(prim_path, "high", time)
                .map(f64::from)
                .unwrap_or(close);
            let low = self
                .temporal_stage
                .resolve_attribute_at(prim_path, "low", time)
                .map(f64::from)
                .unwrap_or(close);
            let volume = self
                .temporal_stage
                .resolve_attribute_at(prim_path, "volume", time)
                .map(f64::from)
                .unwrap_or(0.0);
            window.push_bar(open, high, low, close, volume);
        }
        Some(window)
    }
}

impl MarketProviderServices for ProductionStageProvider {
    fn sample_timeline(&self, path: &str, start_time: f64, end_time: f64) -> Vec<Vector> {
        let Some((prim_path, attribute)) = parse_stage_attribute_path(path) else {
            return Vec::new();
        };

        let active = self
            .usd_stage
            .with_stage(|stage| {
                Ok(stage
                    .field::<bool>(prim_path, FieldKey::Active)
                    .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?
                    .unwrap_or(true))
            })
            .unwrap_or(true);

        if !active {
            return Vec::new();
        }

        self.temporal_stage
            .samples_in_time_range(prim_path, attribute, start_time, end_time)
            .into_iter()
            .map(|(_, value)| Vector::scalar(f64::from(value)))
            .collect()
    }

    fn get_global_attribute(&self, name: &str, t: f64) -> Option<Vector> {
        if name.starts_with("global::") {
            return self.resolve_global_metadata(name);
        }

        match name {
            "close" => self.resolve_scalar_attribute(&self.active_path, t),
            "open" | "high" | "low" | "volume" => {
                let (prim_path, _) = parse_stage_attribute_path(&self.active_path)?;
                if !self.prim_active(prim_path) {
                    return None;
                }
                self.temporal_stage
                    .resolve_attribute_at(prim_path, name, t)
                    .map(f64::from)
                    .filter(|value| value.is_finite())
                    .map(Vector::scalar)
            }
            "portfolio::cash" => self
                .temporal_stage
                .resolve_attribute_at(EXECUTION_CASH_PATH, EXECUTION_CASH_ATTR, t)
                .map(f64::from)
                .filter(|value| value.is_finite())
                .map(Vector::scalar),
            path if path.starts_with('/') => self.resolve_scalar_attribute(path, t),
            _ => None,
        }
    }

    fn execute_integrator(
        &self,
        integrator_name: &str,
        inputs: &[OtlClosure],
        t: f64,
    ) -> Option<Vector> {
        if let Some(value) = execute_stdlib_integrator(self, integrator_name, inputs, t) {
            return Some(value);
        }
        if integrator_name.starts_with("financial::") {
            return execute_financial_integrator(
                self,
                integrator_name,
                &self.active_path,
                inputs,
                t,
            );
        }
        match integrator_name {
            "identity" => inputs.first().and_then(|closure| closure(self, t)),
            "sum" => {
                let total: f64 = inputs
                    .iter()
                    .filter_map(|closure| closure(self, t))
                    .filter_map(|vector| vector.as_scalar())
                    .sum();
                Some(Vector::scalar(total))
            }
            "vector_ta::rsi" => {
                let period = inputs
                    .first()?
                    .clone()(self, t)?
                    .as_scalar()?
                    .round()
                    .max(1.0) as usize;
                let window = self.build_close_window(t, period.saturating_add(5))?;
                compute_ta_latest_with_params("rsi", &window, period).map(Vector::scalar)
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stage_bridge::usd_spike::fixture_path;
    use crate::trading_stage::asset_prim_path;

    fn seeded_temporal_stage() -> Arc<MarketStage> {
        let mut stage = MarketStage::new();
        let prim = asset_prim_path("SPY").unwrap();
        for (time, price) in [(0.0, 100.0), (1.0, 102.0), (2.0, 101.0), (3.0, 105.0), (4.0, 104.0)]
        {
            stage
                .set_sample(&prim, "close", time, price as f32)
                .unwrap();
        }
        Arc::new(stage)
    }

    fn provider_from_fixture() -> ProductionStageProvider {
        let usd = Arc::new(
            UsdStageBridge::open(fixture_path("spy_assets.usda")).expect("open spy fixture"),
        );
        ProductionStageProvider::new(usd, seeded_temporal_stage(), "/MarketLab/SPY/close")
    }

    #[test]
    fn sample_timeline_reads_contiguous_stage_slice() {
        let provider = provider_from_fixture();
        let samples = provider.sample_timeline("/MarketLab/SPY/close", 1.0, 3.0);
        assert_eq!(samples.len(), 3);
        assert_eq!(samples[0].as_scalar(), Some(102.0));
    }

    #[test]
    fn sample_timeline_empty_when_usd_deactivates_prim() {
        let usd = Arc::new(
            UsdStageBridge::open(fixture_path("spy_assets_inactive_overlay.usda"))
                .expect("open inactive overlay"),
        );
        let provider =
            ProductionStageProvider::new(usd, seeded_temporal_stage(), "/MarketLab/SPY/close");
        assert!(provider.sample_timeline("/MarketLab/SPY/close", 0.0, 4.0).is_empty());
    }

    #[test]
    fn get_global_attribute_resolves_close_at_playhead() {
        let provider = provider_from_fixture();
        let close = provider.get_global_attribute("close", 4.0).unwrap();
        assert_eq!(close.as_scalar(), Some(104.0));
    }

    #[test]
    fn get_global_attribute_reads_usd_metadata_prefix() {
        let provider = provider_from_fixture();
        let budget = provider
            .get_global_attribute("global::risk_budget", 0.0)
            .unwrap();
        assert_eq!(budget.as_scalar(), Some(1.0));
    }

    #[test]
    fn provider_is_send_and_sync() {
        let provider = provider_from_fixture();
        fn assert_send_sync<T: Send + Sync>(_: &T) {}
        assert_send_sync(&provider);
    }
}
