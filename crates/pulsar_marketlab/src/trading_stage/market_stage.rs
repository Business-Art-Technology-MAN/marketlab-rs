//! Phase B Pillar 1 — OpenUSD-inspired in-memory market stage.
//!
//! Hierarchical path-addressable prims with time-sampled attributes keyed by
//! continuous `f64` coordinates. Queries use causal forward-fill (hold previous
//! sample) with zero future look-ahead.

use std::collections::{BTreeMap, HashMap};
use std::fmt;

use chrono::{NaiveDate, TimeZone, Utc};
use ordered_float::OrderedFloat;

/// Invalid hierarchical path passed to stage mutators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarketStagePathError {
    InvalidPath,
}

impl fmt::Display for MarketStagePathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MarketStagePathError::InvalidPath => write!(
                f,
                "path must be a non-empty slash-delimited absolute path without '//' segments"
            ),
        }
    }
}

impl std::error::Error for MarketStagePathError {}

pub(crate) fn validate_stage_path(path: &str) -> Result<(), MarketStagePathError> {
    if !path.starts_with('/') || path.len() < 2 {
        return Err(MarketStagePathError::InvalidPath);
    }
    if path.ends_with('/') {
        return Err(MarketStagePathError::InvalidPath);
    }
    for segment in path.split('/').skip(1) {
        if segment.is_empty() {
            return Err(MarketStagePathError::InvalidPath);
        }
    }
    Ok(())
}

/// Unix epoch seconds for a Yahoo-style `YYYY-MM-DD` daily bar.
///
/// Uses UTC 20:00 on the calendar date (approximate US cash-session close).
pub fn stage_time_from_bar_date(date: &str) -> Option<f64> {
    let (year, rest) = date.trim().split_once('-')?;
    let (month, day) = rest.split_once('-')?;
    let y: i32 = year.parse().ok()?;
    let m: u32 = month.parse().ok()?;
    let d: u32 = day.parse().ok()?;
    let naive = NaiveDate::from_ymd_opt(y, m, d)?.and_hms_opt(20, 0, 0)?;
    Some(Utc.from_utc_datetime(&naive).timestamp() as f64)
}

/// Build a canonical asset prim path: `/assets/{ticker}`.
pub fn asset_prim_path(ticker: &str) -> Result<String, MarketStagePathError> {
    let ticker = ticker.trim();
    if ticker.is_empty() || ticker.contains('/') {
        return Err(MarketStagePathError::InvalidPath);
    }
    let path = format!("/assets/{ticker}");
    validate_stage_path(&path)?;
    Ok(path)
}

/// Build a canonical analytics prim path: `/analytics/{indicator_id}`.
pub fn analytics_prim_path(indicator_id: &str) -> Result<String, MarketStagePathError> {
    let indicator_id = indicator_id.trim();
    if indicator_id.is_empty() || indicator_id.contains('/') {
        return Err(MarketStagePathError::InvalidPath);
    }
    let path = format!("/analytics/{indicator_id}");
    validate_stage_path(&path)?;
    Ok(path)
}

/// Sparse time series stored as sorted `(timestamp → value)` samples.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TimeSampledAttribute {
    pub samples: BTreeMap<OrderedFloat<f64>, f32>,
}

impl TimeSampledAttribute {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_sample(&mut self, time: f64, value: f32) {
        if !time.is_finite() || !value.is_finite() {
            return;
        }
        self.samples.insert(OrderedFloat(time), value);
    }

    /// Causal forward-fill: exact match or nearest sample at or before `t`.
    pub fn evaluate_at_time(&self, t: f64) -> Option<f32> {
        if !t.is_finite() {
            return None;
        }
        self.samples
            .range(..=OrderedFloat(t))
            .next_back()
            .map(|(_, value)| *value)
    }

    pub fn earliest_time(&self) -> Option<f64> {
        self.samples.keys().next().map(|key| key.into_inner())
    }

    pub fn latest_time(&self) -> Option<f64> {
        self.samples.keys().next_back().map(|key| key.into_inner())
    }
}

/// One stage prim holding named time-sampled attributes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MarketPrim {
    pub attributes: HashMap<String, TimeSampledAttribute>,
}

impl MarketPrim {
    pub fn attribute_mut(&mut self, name: impl Into<String>) -> &mut TimeSampledAttribute {
        self.attributes
            .entry(name.into())
            .or_insert_with(TimeSampledAttribute::new)
    }

    pub fn evaluate_attribute_at(&self, attribute: &str, t: f64) -> Option<f32> {
        self.attributes
            .get(attribute)
            .and_then(|series| series.evaluate_at_time(t))
    }
}

/// Central in-memory market stage mapping hierarchical paths to prims.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MarketStage {
    pub prims: HashMap<String, MarketPrim>,
}

impl MarketStage {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn prim_mut(&mut self, prim_path: &str) -> Result<&mut MarketPrim, MarketStagePathError> {
        validate_stage_path(prim_path)?;
        Ok(self.prims.entry(prim_path.to_string()).or_default())
    }

    pub fn set_sample(
        &mut self,
        prim_path: &str,
        attribute: &str,
        time: f64,
        value: f32,
    ) -> Result<(), MarketStagePathError> {
        let prim = self.prim_mut(prim_path)?;
        prim.attribute_mut(attribute).set_sample(time, value);
        Ok(())
    }

    /// Resolve `(prim_path, attribute)` at continuous time `t`.
    pub fn resolve_attribute_at(
        &self,
        prim_path: &str,
        attribute: &str,
        t: f64,
    ) -> Option<f32> {
        self.prims
            .get(prim_path)
            .and_then(|prim| prim.evaluate_attribute_at(attribute, t))
    }

    /// Resolve a fully qualified path such as `/assets/SPY/close`.
    pub fn resolve_at(&self, full_path: &str, t: f64) -> Option<f32> {
        let (prim_path, attribute) = split_prim_and_attribute(full_path)?;
        self.resolve_attribute_at(prim_path, attribute, t)
    }

    /// Causal samples with timestamps in `[start, end]` (inclusive).
    pub fn samples_in_time_range(
        &self,
        prim_path: &str,
        attribute: &str,
        start: f64,
        end: f64,
    ) -> Vec<(f64, f32)> {
        if !start.is_finite() || !end.is_finite() || start > end {
            return Vec::new();
        }
        let Some(prim) = self.prims.get(prim_path) else {
            return Vec::new();
        };
        let Some(series) = prim.attributes.get(attribute) else {
            return Vec::new();
        };
        series
            .samples
            .range(OrderedFloat(start)..=OrderedFloat(end))
            .map(|(time, value)| (time.into_inner(), *value))
            .collect()
    }

    /// Collect attribute values in `(playhead_time - lookback_duration) ..= playhead_time`.
    pub fn collect_values_in_window(
        &self,
        prim_path: &str,
        attribute: &str,
        playhead_time: f64,
        lookback_duration_secs: f64,
    ) -> Vec<f64> {
        if !playhead_time.is_finite() || !lookback_duration_secs.is_finite() || lookback_duration_secs <= 0.0
        {
            return Vec::new();
        }
        let start = playhead_time - lookback_duration_secs;
        self.samples_in_time_range(prim_path, attribute, start, playhead_time)
            .into_iter()
            .map(|(_, value)| f64::from(value))
            .collect()
    }
}

fn split_prim_and_attribute(full_path: &str) -> Option<(&str, &str)> {
    validate_stage_path(full_path).ok()?;
    let slash = full_path.rfind('/')?;
    if slash == 0 {
        return None;
    }
    let attribute = &full_path[slash + 1..];
    if attribute.is_empty() {
        return None;
    }
    Some((&full_path[..slash], attribute))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evaluate_at_time_returns_exact_match() {
        let mut series = TimeSampledAttribute::new();
        series.set_sample(10.0, 1.5);
        assert_eq!(series.evaluate_at_time(10.0), Some(1.5));
    }

    #[test]
    fn evaluate_at_time_forward_fills_from_prior_sample() {
        let mut series = TimeSampledAttribute::new();
        series.set_sample(10.0, 1.0);
        series.set_sample(20.0, 2.0);
        assert_eq!(series.evaluate_at_time(15.0), Some(1.0));
        assert_eq!(series.evaluate_at_time(20.0), Some(2.0));
        assert_eq!(series.evaluate_at_time(25.0), Some(2.0));
    }

    #[test]
    fn evaluate_at_time_returns_none_before_first_sample() {
        let mut series = TimeSampledAttribute::new();
        series.set_sample(10.0, 1.0);
        assert_eq!(series.evaluate_at_time(9.0), None);
    }

    #[test]
    fn evaluate_at_time_is_causal() {
        let mut series = TimeSampledAttribute::new();
        series.set_sample(10.0, 1.0);
        series.set_sample(20.0, 2.0);
        assert_eq!(series.evaluate_at_time(9.0), None);
        assert_ne!(series.evaluate_at_time(9.0), Some(2.0));
    }

    #[test]
    fn resolve_attribute_at_uses_stage_paths() {
        let mut stage = MarketStage::new();
        let prim = asset_prim_path("SPY").unwrap();
        stage.set_sample(&prim, "close", 100.0, 420.0).unwrap();
        stage.set_sample(&prim, "close", 200.0, 430.0).unwrap();
        assert_eq!(stage.resolve_attribute_at(&prim, "close", 150.0), Some(420.0));
        assert_eq!(stage.resolve_at("/assets/SPY/close", 200.0), Some(430.0));
    }

    #[test]
    fn stage_time_from_bar_date_is_monotonic() {
        let early = stage_time_from_bar_date("2024-01-02").unwrap();
        let late = stage_time_from_bar_date("2024-02-01").unwrap();
        assert!(late > early);
    }

    #[test]
    fn stage_time_from_bar_date_uses_unix_epoch_seconds() {
        let t = stage_time_from_bar_date("2024-01-02").unwrap();
        let expected = NaiveDate::from_ymd_opt(2024, 1, 2)
            .unwrap()
            .and_hms_opt(20, 0, 0)
            .unwrap();
        let expected = Utc.from_utc_datetime(&expected).timestamp() as f64;
        assert!((t - expected).abs() < f64::EPSILON);
        assert!(t > 1_700_000_000.0);
    }

    #[test]
    fn samples_in_time_range_is_causal_and_inclusive() {
        let mut stage = MarketStage::new();
        let prim = asset_prim_path("SPY").unwrap();
        stage.set_sample(&prim, "close", 100.0, 1.0).unwrap();
        stage.set_sample(&prim, "close", 200.0, 2.0).unwrap();
        stage.set_sample(&prim, "close", 300.0, 3.0).unwrap();
        let samples = stage.samples_in_time_range(&prim, "close", 150.0, 250.0);
        assert_eq!(samples, vec![(200.0, 2.0)]);
        let window = stage.collect_values_in_window(&prim, "close", 250.0, 100.0);
        assert_eq!(window, vec![2.0]);
    }

    #[test]
    fn invalid_paths_are_rejected() {
        assert!(validate_stage_path("assets/SPY").is_err());
        assert!(validate_stage_path("/assets//SPY").is_err());
        assert!(asset_prim_path("").is_err());
    }
}
