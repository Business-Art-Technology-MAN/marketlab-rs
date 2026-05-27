//! Shared mock provider for OTL closure tests.

use std::collections::HashMap;

use super::services::MarketProviderServices;
use super::vector::Vector;

pub struct MockMarketProvider {
    timeline: HashMap<String, Vec<(f64, Vector)>>,
    globals: HashMap<String, Vector>,
    active_close_path: String,
}

impl MockMarketProvider {
    pub fn with_price_path(path: &str, samples: &[(f64, f64)]) -> Self {
        let mut timeline = HashMap::new();
        timeline.insert(
            path.to_string(),
            samples
                .iter()
                .map(|(time, price)| (*time, Vector::scalar(*price)))
                .collect(),
        );
        Self {
            timeline,
            globals: HashMap::from([(
                "portfolio::cash".to_string(),
                Vector::scalar(10_000.0),
            )]),
            active_close_path: path.to_string(),
        }
    }

    pub fn point_at_playhead(&self, t: f64) -> Option<Vector> {
        self.timeline
            .get(&self.active_close_path)?
            .iter()
            .filter(|(time, _)| *time <= t)
            .next_back()
            .map(|(_, value)| value.clone())
    }
}

impl MarketProviderServices for MockMarketProvider {
    fn sample_timeline(&self, path: &str, start_time: f64, end_time: f64) -> Vec<Vector> {
        let Some(series) = self.timeline.get(path) else {
            return Vec::new();
        };
        series
            .iter()
            .filter(|(time, _)| *time >= start_time && *time <= end_time)
            .map(|(_, value)| value.clone())
            .collect()
    }

    fn get_global_attribute(&self, name: &str, t: f64) -> Option<Vector> {
        if let Some(value) = self.globals.get(name) {
            return Some(value.clone());
        }
        match name {
            "close" | "open" | "high" | "low" | "volume" => {
                let close = self.point_at_playhead(t)?.as_scalar()?;
                let value = match name {
                    "close" | "open" => close,
                    "high" => close * 1.01,
                    "low" => close * 0.99,
                    "volume" => 0.0,
                    _ => return None,
                };
                Some(Vector::scalar(value))
            }
            _ => None,
        }
    }

    fn execute_integrator(
        &self,
        integrator_name: &str,
        inputs: &[super::services::OtlClosure],
        t: f64,
    ) -> Option<Vector> {
        if let Some(value) =
            super::financial::execute_stdlib_integrator(self, integrator_name, inputs, t)
        {
            return Some(value);
        }
        if integrator_name.starts_with("financial::") {
            return super::financial::execute_financial_integrator(
                self,
                integrator_name,
                &self.active_close_path,
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
                let start = t - period as f64 + 1.0;
                let samples = self.sample_timeline(&self.active_close_path, start, t);
                if samples.len() < 2 {
                    return None;
                }
                let closes: Vec<f64> = samples.iter().filter_map(|v| v.as_scalar()).collect();
                let mut gains = 0.0;
                let mut losses = 0.0;
                for pair in closes.windows(2) {
                    let delta = pair[1] - pair[0];
                    if delta >= 0.0 {
                        gains += delta;
                    } else {
                        losses -= delta;
                    }
                }
                let avg_gain = gains / period as f64;
                let avg_loss = losses / period as f64;
                if avg_loss <= f64::EPSILON {
                    return Some(Vector::scalar(100.0));
                }
                let rs = avg_gain / avg_loss;
                Some(Vector::scalar(100.0 - (100.0 / (1.0 + rs))))
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    #[test]
    fn sample_timeline_returns_deterministic_price_sequence() {
        let provider = MockMarketProvider::with_price_path(
            "/assets/active/close",
            &[(1.0, 100.0), (2.0, 101.5), (3.0, 99.0)],
        );
        let samples = provider.sample_timeline("/assets/active/close", 1.0, 2.0);
        assert_eq!(samples.len(), 2);
        assert_eq!(samples[0].as_scalar(), Some(100.0));
        assert_eq!(samples[1].as_scalar(), Some(101.5));
    }

    #[test]
    fn get_global_attribute_reads_framing_properties() {
        let provider = MockMarketProvider::with_price_path("/assets/active/close", &[]);
        let cash = provider
            .get_global_attribute("portfolio::cash", 1_706_000_000.0)
            .expect("cash");
        assert_eq!(cash.as_scalar(), Some(10_000.0));
    }

    #[test]
    fn execute_integrator_routes_closure_inputs() {
        let provider = MockMarketProvider::with_price_path(
            "/assets/active/close",
            &[(1.0, 420.0)],
        );
        let cash_reader: super::super::services::OtlClosure = Arc::new(|services, t| {
            services.get_global_attribute("portfolio::cash", t)
        });
        let price_reader: super::super::services::OtlClosure = Arc::new(|services, _| {
            services
                .sample_timeline("/assets/active/close", 1.0, 1.0)
                .into_iter()
                .next()
        });

        let identity = provider.execute_integrator("identity", &[cash_reader.clone()], 0.0);
        assert_eq!(identity.and_then(|v| v.as_scalar()), Some(10_000.0));

        let summed = provider.execute_integrator("sum", &[cash_reader, price_reader], 1.0);
        assert_eq!(summed.and_then(|v| v.as_scalar()), Some(10_420.0));
    }
}
