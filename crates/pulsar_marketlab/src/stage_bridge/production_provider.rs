//! Live [`MarketStage`] adapter for OTL closure execution.

use crate::execution_engine::{EXECUTION_CASH_ATTR, EXECUTION_CASH_PATH};
use crate::signal_dsl::{MarketProviderServices, OtlClosure, Vector};
use crate::technical_analysis::{compute_ta_latest_with_params, MarketSeriesWindow};
use crate::trading_stage::MarketStage;

use super::parse_stage_attribute_path;

/// Routes OTL evaluation against a real in-memory [`MarketStage`].
#[derive(Clone, Debug)]
pub struct ProductionStageProvider<'a> {
    stage: &'a MarketStage,
    active_close_path: String,
}

impl<'a> ProductionStageProvider<'a> {
    pub fn new(stage: &'a MarketStage, active_close_path: impl Into<String>) -> Self {
        Self {
            stage,
            active_close_path: active_close_path.into(),
        }
    }

    pub fn stage(&self) -> &MarketStage {
        self.stage
    }

    pub fn active_close_path(&self) -> &str {
        &self.active_close_path
    }

    fn resolve_scalar_attribute(&self, full_path: &str, t: f64) -> Option<Vector> {
        let (prim_path, attribute) = parse_stage_attribute_path(full_path)?;
        self.stage
            .resolve_attribute_at(prim_path, attribute, t)
            .map(f64::from)
            .filter(|value| value.is_finite())
            .map(Vector::scalar)
    }

    fn build_close_window(&self, t: f64, bar_count: usize) -> Option<MarketSeriesWindow> {
        let (prim_path, _) = parse_stage_attribute_path(&self.active_close_path)?;
        if bar_count == 0 {
            return None;
        }
        let start = t - bar_count as f64 + 1.0;
        let samples = self
            .stage
            .samples_in_time_range(prim_path, "close", start, t);
        if samples.is_empty() {
            return None;
        }
        let mut window = MarketSeriesWindow::default();
        for (time, close) in samples {
            let close = f64::from(close);
            let open = self
                .stage
                .resolve_attribute_at(prim_path, "open", time)
                .map(f64::from)
                .unwrap_or(close);
            let high = self
                .stage
                .resolve_attribute_at(prim_path, "high", time)
                .map(f64::from)
                .unwrap_or(close);
            let low = self
                .stage
                .resolve_attribute_at(prim_path, "low", time)
                .map(f64::from)
                .unwrap_or(close);
            let volume = self
                .stage
                .resolve_attribute_at(prim_path, "volume", time)
                .map(f64::from)
                .unwrap_or(0.0);
            window.push_bar(open, high, low, close, volume);
        }
        Some(window)
    }
}

impl MarketProviderServices for ProductionStageProvider<'_> {
    fn sample_timeline(&self, path: &str, start_time: f64, end_time: f64) -> Vec<Vector> {
        let Some((prim_path, attribute)) = parse_stage_attribute_path(path) else {
            return Vec::new();
        };
        self.stage
            .samples_in_time_range(prim_path, attribute, start_time, end_time)
            .into_iter()
            .map(|(_, value)| Vector::scalar(f64::from(value)))
            .collect()
    }

    fn get_global_attribute(&self, name: &str, t: f64) -> Option<Vector> {
        match name {
            "close" => self.resolve_scalar_attribute(&self.active_close_path, t),
            "open" | "high" | "low" | "volume" => {
                let (prim_path, _) = parse_stage_attribute_path(&self.active_close_path)?;
                self.stage
                    .resolve_attribute_at(prim_path, name, t)
                    .map(f64::from)
                    .filter(|value| value.is_finite())
                    .map(Vector::scalar)
            }
            "portfolio::cash" => self
                .stage
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
    use crate::trading_stage::asset_prim_path;

    fn seeded_stage() -> MarketStage {
        let mut stage = MarketStage::new();
        let prim = asset_prim_path("SPY").unwrap();
        for (time, price) in [(0.0, 100.0), (1.0, 102.0), (2.0, 101.0), (3.0, 105.0), (4.0, 104.0)]
        {
            stage
                .set_sample(&prim, "close", time, price as f32)
                .unwrap();
        }
        stage
    }

    #[test]
    fn sample_timeline_reads_contiguous_stage_slice() {
        let stage = seeded_stage();
        let provider = ProductionStageProvider::new(&stage, "/assets/SPY/close");
        let samples = provider.sample_timeline("/assets/SPY/close", 1.0, 3.0);
        assert_eq!(samples.len(), 3);
        assert_eq!(samples[0].as_scalar(), Some(102.0));
    }

    #[test]
    fn get_global_attribute_resolves_close_at_playhead() {
        let stage = seeded_stage();
        let provider = ProductionStageProvider::new(&stage, "/assets/SPY/close");
        let close = provider.get_global_attribute("close", 4.0).unwrap();
        assert_eq!(close.as_scalar(), Some(104.0));
    }

    #[test]
    fn provider_is_send_and_sync() {
        let stage = seeded_stage();
        let provider = ProductionStageProvider::new(&stage, "/assets/SPY/close");
        fn assert_send_sync<T: Send + Sync>(_: &T) {}
        assert_send_sync(&provider);
    }
}
