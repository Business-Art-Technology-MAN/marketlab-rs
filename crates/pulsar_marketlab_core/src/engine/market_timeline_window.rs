//! Bar-indexed market access for graph sweeps (strict lookback + exact prim-path keys).

use std::collections::HashMap;
use std::sync::Arc;

/// Zero-copy view into a shared close-price column with optional frame windowing.
#[derive(Clone, Debug)]
pub struct SharedPriceColumn {
    data: Arc<[f64]>,
    offset: usize,
    len: usize,
}

impl SharedPriceColumn {
    pub fn from_series(data: Arc<[f64]>) -> Self {
        let len = data.len();
        Self {
            data,
            offset: 0,
            len,
        }
    }

    pub fn windowed(data: Arc<[f64]>, in_frame: usize, out_frame: usize) -> Self {
        if data.is_empty() {
            return Self {
                data,
                offset: 0,
                len: 0,
            };
        }
        let end = out_frame.min(data.len() - 1);
        let start = in_frame.min(end);
        Self {
            data,
            offset: start,
            len: end - start + 1,
        }
    }

    pub fn as_slice(&self) -> &[f64] {
        if self.len == 0 {
            return &[];
        }
        &self.data[self.offset..self.offset + self.len]
    }

    pub fn full_data(&self) -> &Arc<[f64]> {
        &self.data
    }

    /// Materialize a padded timeline column only when length or offset differs from the target.
    pub fn into_timeline_arc(self, timeline_len: usize) -> Arc<[f64]> {
        let slice = self.as_slice();
        if slice.len() == timeline_len && self.offset == 0 && self.data.len() == timeline_len {
            return self.data;
        }
        Arc::from(pad_series(slice.to_vec(), timeline_len).into_boxed_slice())
    }
}

/// Side-channel hook for parse-safe `outputs:weights` token emission during bar evaluation.
pub type WeightTrackCallback = Arc<dyn Fn(usize, &str, &str) + Send + Sync>;

/// Read-only timeline view used during [`super::vector_sweep`] and [`crate::MarketLabGraphEngine::sweep`].
///
/// All heap allocation happens in [`Self::activate`]; the sweep hot path only performs
/// map lookups by exact OpenUSD prim path string (or pre-resolved slot index).
#[derive(Clone)]
pub struct MarketTimelineWindow {
    timeline_len: usize,
    /// Keys are absolute prim paths exactly as emitted by canvas compose (e.g. `/MarketLab/SPY`).
    price_vectors: Arc<HashMap<Arc<str>, Arc<[f64]>>>,
    path_to_slot: Arc<HashMap<Arc<str>, usize>>,
    slot_prices: Arc<Vec<Arc<[f64]>>>,
    /// Active bar index for deferred OTL closure evaluation (set by sweep driver).
    current_frame: usize,
    /// Optional runtime collector: `(frame, portfolio_prim_path, encoded_weights)`.
    track_token: Option<WeightTrackCallback>,
}

impl MarketTimelineWindow {
    /// Build padded price columns once before timeline activation (allocations allowed here).
    pub fn activate(
        asset_vectors: HashMap<String, SharedPriceColumn>,
        timeline_len: usize,
    ) -> Self {
        let mut path_to_slot = HashMap::new();
        let mut slot_prices = Vec::new();

        for (path, column) in asset_vectors {
            let key: Arc<str> = Arc::from(path.as_str());
            let padded = column.into_timeline_arc(timeline_len);
            let slot = slot_prices.len();
            path_to_slot.insert(Arc::clone(&key), slot);
            slot_prices.push(padded);
        }

        let mut price_vectors = HashMap::new();
        for (path, slot) in path_to_slot.iter() {
            if let Some(series) = slot_prices.get(*slot) {
                price_vectors.insert(Arc::clone(path), Arc::clone(series));
            }
        }

        Self {
            timeline_len,
            price_vectors: Arc::new(price_vectors),
            path_to_slot: Arc::new(path_to_slot),
            slot_prices: Arc::new(slot_prices),
            current_frame: 0,
            track_token: None,
        }
    }

    /// Attach a side-channel collector (allocations only at activation, not per bar).
    pub fn with_weight_tracker(mut self, track_token: WeightTrackCallback) -> Self {
        self.track_token = Some(track_token);
        self
    }

    pub fn set_current_frame(&mut self, frame: usize) {
        self.current_frame = frame.min(self.timeline_len.saturating_sub(1));
    }

    pub fn current_frame(&self) -> usize {
        self.current_frame
    }

    /// Emit a parse-safe `outputs:weights` snapshot for the active frame.
    pub fn track_token(&self, portfolio_prim_path: &str, encoded_weights: &str) {
        if let Some(callback) = &self.track_token {
            callback(self.current_frame, portfolio_prim_path, encoded_weights);
        }
    }

    pub fn timeline_len(&self) -> usize {
        self.timeline_len
    }

    pub fn price_vectors(&self) -> &HashMap<Arc<str>, Arc<[f64]>> {
        &self.price_vectors
    }

    /// Price at the active [`Self::current_frame`] (deferred closure helper).
    pub fn get_price_at_frame(&self, prim_path: &str, frame_offset: isize) -> f64 {
        let frame = self.current_frame as isize + frame_offset;
        self.price_at_path(prim_path, frame)
    }

    /// Exact prim-path lookup (no normalization, no allocation).
    pub fn series_at_path(&self, prim_path: &str) -> Option<&[f64]> {
        self.price_vectors
            .get(prim_path)
            .map(|series| series.as_ref())
    }

    /// O(1) slot lookup when the prim path was registered at activation.
    pub fn series_at_slot(&self, slot: usize) -> Option<&[f64]> {
        self.slot_prices.get(slot).map(|series| series.as_ref())
    }

    pub fn slot_for_path(&self, prim_path: &str) -> Option<usize> {
        self.path_to_slot.get(prim_path).copied()
    }

    /// Resolve price at `frame` with strict lookback isolation.
    ///
    /// Frames before `0` return `0.0`. Frames at or beyond `timeline_len` return `0.0`.
    pub fn price_at_path(&self, prim_path: &str, frame: isize) -> f64 {
        self.price_at_path_opt(prim_path, frame).unwrap_or(0.0)
    }

    /// Same as [`Self::price_at_path`] but returns `None` when the path is unknown or frame &lt; 0.
    pub fn price_at_path_opt(&self, prim_path: &str, frame: isize) -> Option<f64> {
        if frame < 0 {
            return None;
        }
        let frame = frame as usize;
        if frame >= self.timeline_len {
            return None;
        }
        let series = self.series_at_path(prim_path)?;
        series.get(frame).copied().filter(|value| value.is_finite())
    }

    /// Slot-based price lookup for binding tables (no string hash in inner loops).
    pub fn price_at_slot(&self, slot: usize, frame: isize) -> f64 {
        self.price_at_slot_opt(slot, frame).unwrap_or(0.0)
    }

    pub fn price_at_slot_opt(&self, slot: usize, frame: isize) -> Option<f64> {
        if frame < 0 {
            return None;
        }
        let frame = frame as usize;
        if frame >= self.timeline_len {
            return None;
        }
        let series = self.series_at_slot(slot)?;
        series.get(frame).copied().filter(|value| value.is_finite())
    }
}

/// Build shared columns from owned vectors (used at integration boundaries and tests).
pub fn shared_columns_from_vec(
    vectors: HashMap<String, Vec<f64>>,
) -> HashMap<String, SharedPriceColumn> {
    vectors
        .into_iter()
        .map(|(path, series)| {
            (
                path,
                SharedPriceColumn::from_series(Arc::from(series.into_boxed_slice())),
            )
        })
        .collect()
}

fn pad_series(mut series: Vec<f64>, timeline_len: usize) -> Vec<f64> {
    if series.len() == timeline_len {
        return series;
    }
    if series.len() > timeline_len {
        series.truncate(timeline_len);
        return series;
    }
    series.resize(timeline_len, 0.0);
    series
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn negative_frame_returns_none_or_zero() {
        let window = MarketTimelineWindow::activate(
            shared_columns_from_vec(HashMap::from([(
                "/MarketLab/SPY".to_string(),
                vec![1.0, 2.0, 3.0],
            )])),
            3,
        );
        assert_eq!(window.price_at_path_opt("/MarketLab/SPY", -1), None);
        assert_eq!(window.price_at_path("/MarketLab/SPY", -1), 0.0);
        assert_eq!(window.price_at_path("/MarketLab/SPY", 1), 2.0);
    }

    #[test]
    fn keys_require_exact_prim_path() {
        let window = MarketTimelineWindow::activate(
            shared_columns_from_vec(HashMap::from([(
                "/MarketLab/SPY".to_string(),
                vec![9.0],
            )])),
            1,
        );
        assert!(window.series_at_path("/MarketLab/SPY").is_some());
        assert!(window.series_at_path("SPY").is_none());
        assert!(window.series_at_path("/MarketLab/SPY/").is_none());
    }
}
