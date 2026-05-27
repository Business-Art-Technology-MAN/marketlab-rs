//! OpenUSD composition spike — profiles time-sample retrieval and layer stacking.
//!
//! This module provides an in-memory Sdf/Pcp layer analog that runs in CI without
//! native Pixar USD libraries. Enable `--features openusd-spike` to probe optional
//! `rust-usd` bindings when a local OpenUSD install is available.

use std::collections::{BTreeMap, HashMap};
use std::time::{Duration, Instant};

use ordered_float::OrderedFloat;

use crate::signal_dsl::Vector;

/// One authored layer mapping attribute paths to time samples.
#[derive(Clone, Debug, Default)]
pub struct UsdAuthoredLayer {
    pub samples: HashMap<String, BTreeMap<OrderedFloat<f64>, f32>>,
}

impl UsdAuthoredLayer {
    pub fn insert_sample(&mut self, path: impl Into<String>, time: f64, value: f32) {
        if !time.is_finite() || !value.is_finite() {
            return;
        }
        self.samples
            .entry(path.into())
            .or_default()
            .insert(OrderedFloat(time), value);
    }
}

/// Minimal Pcp-style layer stack with strong-to-weak composition order.
#[derive(Clone, Debug, Default)]
pub struct UsdLayerStack {
    layers: Vec<UsdAuthoredLayer>,
}

impl UsdLayerStack {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push_layer(&mut self, layer: UsdAuthoredLayer) {
        self.layers.push(layer);
    }

    /// Resolve an attribute at `t` by walking layers strongest-first.
    pub fn resolve_at(&self, path: &str, t: f64) -> Option<f32> {
        if !t.is_finite() {
            return None;
        }
        for layer in self.layers.iter().rev() {
            if let Some(series) = layer.samples.get(path) {
                if let Some((_, value)) = series.range(..=OrderedFloat(t)).next_back() {
                    return Some(*value);
                }
            }
        }
        None
    }

    /// Collect causal samples in `[start, end]` after layer composition.
    pub fn samples_in_range(&self, path: &str, start: f64, end: f64) -> Vec<(f64, f32)> {
        if !start.is_finite() || !end.is_finite() || start > end {
            return Vec::new();
        }
        let mut merged: BTreeMap<OrderedFloat<f64>, f32> = BTreeMap::new();
        for layer in &self.layers {
            if let Some(series) = layer.samples.get(path) {
                for (time, value) in series.range(OrderedFloat(start)..=OrderedFloat(end)) {
                    merged.insert(*time, *value);
                }
            }
        }
        merged
            .into_iter()
            .map(|(time, value)| (time.into_inner(), value))
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct UsdSpikeProfile {
    pub sample_count: usize,
    pub query_count: usize,
    pub resolve_elapsed: Duration,
    pub range_elapsed: Duration,
}

/// Profile thousands of point resolves and range queries across a composed layer stack.
pub fn profile_time_sample_retrieval(sample_count: usize, query_count: usize) -> UsdSpikeProfile {
    let mut base = UsdAuthoredLayer::default();
    for index in 0..sample_count {
        let time = index as f64;
        base.insert_sample("/assets/SPY/close", time, 100.0 + index as f32 * 0.01);
    }

    let mut overlay = UsdAuthoredLayer::default();
    for index in (sample_count / 2)..sample_count {
        let time = index as f64;
        overlay.insert_sample("/assets/SPY/close", time, 200.0 + index as f32 * 0.01);
    }

    let mut stack = UsdLayerStack::new();
    stack.push_layer(base);
    stack.push_layer(overlay);

    let resolve_start = Instant::now();
    for query in 0..query_count {
        let t = (query % sample_count) as f64;
        let _ = stack.resolve_at("/assets/SPY/close", t);
    }
    let resolve_elapsed = resolve_start.elapsed();

    let range_start = Instant::now();
    for query in 0..query_count {
        let end = (query % sample_count) as f64;
        let start = (end - 64.0).max(0.0);
        let _ = stack.samples_in_range("/assets/SPY/close", start, end);
    }
    let range_elapsed = range_start.elapsed();

    UsdSpikeProfile {
        sample_count,
        query_count,
        resolve_elapsed,
        range_elapsed,
    }
}

/// Smoke test that overlapping layers compose like a USD strong opinion overlay.
pub fn composed_close_at(path: &str, t: f64, stack: &UsdLayerStack) -> Option<Vector> {
    stack.resolve_at(path, t).map(|value| Vector::scalar(f64::from(value)))
}

#[cfg(feature = "openusd-spike")]
pub mod native {
    //! Optional native OpenUSD probe via `rust-usd` when locally installed.

    /// Returns `true` when native OpenUSD libraries are linked and reachable.
    pub fn native_openusd_available() -> bool {
        // Placeholder hook — extend with rust-usd stage open when feature is enabled in CI.
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layer_stack_composes_strong_overlay() {
        let mut base = UsdAuthoredLayer::default();
        base.insert_sample("/assets/SPY/close", 10.0, 100.0);
        let mut overlay = UsdAuthoredLayer::default();
        overlay.insert_sample("/assets/SPY/close", 10.0, 111.0);

        let mut stack = UsdLayerStack::new();
        stack.push_layer(base);
        stack.push_layer(overlay);

        assert_eq!(stack.resolve_at("/assets/SPY/close", 10.0), Some(111.0));
        let composed = composed_close_at("/assets/SPY/close", 10.0, &stack).unwrap();
        assert_eq!(composed.as_scalar(), Some(111.0));
    }

    #[test]
    fn profile_retrieval_completes_for_large_sample_sets() {
        let profile = profile_time_sample_retrieval(4_096, 8_192);
        assert_eq!(profile.sample_count, 4_096);
        assert_eq!(profile.query_count, 8_192);
        assert!(profile.resolve_elapsed < Duration::from_secs(5));
        assert!(profile.range_elapsed < Duration::from_secs(5));
    }
}
