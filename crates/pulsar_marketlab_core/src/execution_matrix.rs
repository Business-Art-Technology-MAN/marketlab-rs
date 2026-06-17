//! Shared execution context and column matrix for OTL vector sweeps.

use std::collections::HashMap;
use std::sync::Arc;

use arrow_array::Float64Array;
use arrow_buffer::Buffer;
use thiserror::Error;

use crate::engine::{PrecomputedMatrixCache, RollingMatrixWindow, DEFAULT_COVARIANCE_LOOKBACK};
use crate::engine::WeightTrackCallback;
use crate::AssetQuote;

/// Runtime context for tier vector sweeps (read-only inputs, pre-sized buffers).
#[derive(Clone)]
pub struct ExecutionContext {
    timeline_len: usize,
    pub initial_capital: f64,
    pub allocation_method: String,
    pub asset_quotes: HashMap<String, AssetQuote>,
    pub asset_vectors: HashMap<String, Arc<[f64]>>,
    /// Upstream price/signal window fed into the active signal-tier sweep.
    pub signal_upstream: Vec<f64>,
    /// Matrix column index receiving the active signal-tier output.
    pub signal_output_column: usize,
    /// Deferred side-channel for OTL portfolio `outputs:weights` (set only during tier sweeps).
    pub weight_track: Option<WeightTrackCallback>,
    /// Pre-computed rolling covariance tensor (built once per sweep activation).
    pub covariance_cache: Option<Arc<PrecomputedMatrixCache>>,
    pub covariance_lookback: usize,
}

impl ExecutionContext {
    pub fn new(
        timeline_len: usize,
        initial_capital: f64,
        allocation_method: impl Into<String>,
        asset_quotes: HashMap<String, AssetQuote>,
        asset_vectors: HashMap<String, Arc<[f64]>>,
    ) -> Self {
        Self {
            timeline_len,
            initial_capital,
            allocation_method: allocation_method.into(),
            asset_quotes,
            asset_vectors,
            signal_upstream: Vec::new(),
            signal_output_column: 0,
            weight_track: None,
            covariance_cache: None,
            covariance_lookback: DEFAULT_COVARIANCE_LOOKBACK,
        }
    }

    pub fn set_weight_tracker(&mut self, tracker: Option<WeightTrackCallback>) {
        self.weight_track = tracker;
    }

    pub fn attach_covariance_cache(&mut self, cache: Arc<PrecomputedMatrixCache>) {
        self.covariance_cache = Some(cache);
    }

    pub fn rolling_matrix_window(&self) -> Option<RollingMatrixWindow<'_>> {
        self.covariance_cache
            .as_ref()
            .map(|cache| RollingMatrixWindow::from_cache(cache))
    }

    pub fn timeline_length(&self) -> usize {
        self.timeline_len
    }
}

impl std::fmt::Debug for ExecutionContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExecutionContext")
            .field("timeline_len", &self.timeline_len)
            .field("initial_capital", &self.initial_capital)
            .field("allocation_method", &self.allocation_method)
            .field("weight_track", &self.weight_track.as_ref().map(|_| "<tracker>"))
            .field(
                "covariance_cache",
                &self.covariance_cache.as_ref().map(|cache| cache.total_bars()),
            )
            .field("covariance_lookback", &self.covariance_lookback)
            .finish_non_exhaustive()
    }
}

/// Column-major `columns × bar_count` block (`data[col * bar_count + bar]`).
///
/// Hot-path sweeps use [`Self::column_slice`] / [`Self::column_slice_mut`]. For IPC or
/// cross-process sharing, export via [`Self::column_primitive_array`] or
/// [`Self::flatten_primitive_array`].
#[derive(Debug, Clone)]
pub struct ColumnMajorBlock {
    bar_count: usize,
    columns: usize,
    data: Vec<f64>,
}

impl ColumnMajorBlock {
    pub fn with_shape(bar_count: usize, columns: usize) -> Self {
        Self {
            bar_count,
            columns,
            data: vec![0.0; columns.saturating_mul(bar_count)],
        }
    }

    #[inline]
    fn index(&self, column: usize, bar: usize) -> Option<usize> {
        if column < self.columns && bar < self.bar_count {
            Some(column * self.bar_count + bar)
        } else {
            None
        }
    }

    pub fn bar_count(&self) -> usize {
        self.bar_count
    }

    pub fn columns(&self) -> usize {
        self.columns
    }

    pub fn write(&mut self, column: usize, bar: usize, value: f64) {
        if let Some(index) = self.index(column, bar) {
            self.data[index] = value;
        }
    }

    pub fn read(&self, column: usize, bar: usize) -> Option<f64> {
        self.index(column, bar).map(|index| self.data[index])
    }

    /// Contiguous time series for one column (ideal for cache-friendly SMA sweeps).
    pub fn column_slice(&self, column: usize) -> &[f64] {
        if column >= self.columns {
            return &[];
        }
        let start = column * self.bar_count;
        &self.data[start..start + self.bar_count]
    }

    pub fn column_slice_mut(&mut self, column: usize) -> &mut [f64] {
        assert!(column < self.columns, "column index out of range");
        let start = column * self.bar_count;
        &mut self.data[start..start + self.bar_count]
    }

    pub fn clear_column(&mut self, column: usize) {
        if column < self.columns {
            self.column_slice_mut(column).fill(0.0);
        }
    }

    pub fn clear_all(&mut self) {
        self.data.fill(0.0);
    }

    pub fn copy_column_from_slice(&mut self, column: usize, values: &[f64]) {
        let target = self.column_slice_mut(column);
        let len = target.len().min(values.len());
        target[..len].copy_from_slice(&values[..len]);
        if len < target.len() {
            target[len..].fill(0.0);
        }
    }

    /// Arrow `Float64` view of one column (copies into a new array; use [`Self::shared_buffer`]
    /// for IPC handoff).
    pub fn column_primitive_array(&self, column: usize) -> Float64Array {
        Float64Array::from(self.column_slice(column).to_vec())
    }

    /// Arrow view of the full column-major matrix buffer.
    pub fn flatten_primitive_array(&self) -> Float64Array {
        Float64Array::from(self.data.clone())
    }

    /// Shared Arrow buffer backing this block (clone is cheap; enables IPC without copying f64s).
    pub fn shared_buffer(&self) -> Arc<Buffer> {
        Arc::new(Buffer::from_vec(self.data.clone()))
    }

}

/// Column-oriented workspace for signal, allocator, and portfolio vectors.
#[derive(Debug, Clone)]
pub struct GraphSeriesMatrix {
    bar_count: usize,
    signals: ColumnMajorBlock,
    allocator_weights: ColumnMajorBlock,
    nav: ColumnMajorBlock,
    cash: ColumnMajorBlock,
}

impl GraphSeriesMatrix {
    pub fn with_capacity(bar_count: usize, signal_columns: usize, allocator_legs: usize) -> Self {
        Self {
            bar_count,
            signals: ColumnMajorBlock::with_shape(bar_count, signal_columns),
            allocator_weights: ColumnMajorBlock::with_shape(bar_count, allocator_legs),
            nav: ColumnMajorBlock::with_shape(bar_count, 1),
            cash: ColumnMajorBlock::with_shape(bar_count, 1),
        }
    }

    pub fn bar_count(&self) -> usize {
        self.bar_count
    }

    pub fn write_signal(&mut self, column: usize, bar_idx: usize, value: f64) {
        self.signals.write(column, bar_idx, value);
    }

    pub fn read_signal(&self, column: usize, bar_idx: usize) -> Option<f64> {
        self.signals.read(column, bar_idx)
    }

    pub fn signal_column_slice(&self, column: usize) -> &[f64] {
        self.signals.column_slice(column)
    }

    pub fn signal_column_slice_mut(&mut self, column: usize) -> &mut [f64] {
        self.signals.column_slice_mut(column)
    }

    pub fn signal_column_primitive_array(&self, column: usize) -> Float64Array {
        self.signals.column_primitive_array(column)
    }

    pub fn write_allocator_weight(&mut self, leg: usize, bar_idx: usize, value: f64) {
        self.allocator_weights.write(leg, bar_idx, value);
    }

    pub fn read_allocator_weight(&self, leg: usize, bar_idx: usize) -> Option<f64> {
        self.allocator_weights.read(leg, bar_idx)
    }

    pub fn allocator_leg_slice(&self, leg: usize) -> &[f64] {
        self.allocator_weights.column_slice(leg)
    }

    pub fn write_nav(&mut self, bar_idx: usize, value: f64) {
        self.nav.write(0, bar_idx, value);
    }

    pub fn write_cash(&mut self, bar_idx: usize, value: f64) {
        self.cash.write(0, bar_idx, value);
    }

    pub fn cash_at(&self, bar_idx: usize) -> f64 {
        self.cash.read(0, bar_idx).unwrap_or(0.0)
    }

    pub fn nav_at(&self, bar_idx: usize) -> f64 {
        self.nav.read(0, bar_idx).unwrap_or(0.0)
    }

    pub fn nav_series(&self) -> &[f64] {
        self.nav.column_slice(0)
    }

    pub fn cash_series(&self) -> &[f64] {
        self.cash.column_slice(0)
    }

    pub fn nav_primitive_array(&self) -> Float64Array {
        self.nav.column_primitive_array(0)
    }

    pub fn cash_primitive_array(&self) -> Float64Array {
        self.cash.column_primitive_array(0)
    }

    pub fn signal_column_count(&self) -> usize {
        self.signals.columns()
    }

    pub fn allocator_leg_count(&self) -> usize {
        self.allocator_weights.columns()
    }

    pub fn copy_signal_column_from_slice(&mut self, column: usize, values: &[f64]) {
        self.signals.copy_column_from_slice(column, values);
    }

    pub fn clear_signal_column(&mut self, column: usize) {
        self.signals.clear_column(column);
    }

    pub fn clear_allocator_leg(&mut self, leg: usize) {
        self.allocator_weights.clear_column(leg);
    }

    pub fn clear_portfolio_metrics(&mut self) {
        self.nav.clear_column(0);
        self.cash.clear_column(0);
    }

    pub fn signals_flatten_primitive_array(&self) -> Float64Array {
        self.signals.flatten_primitive_array()
    }
}

#[derive(Debug, Error, PartialEq)]
pub enum RuntimeEngineError {
    #[error("missing signal column index {column_index} in workspace matrix")]
    MissingSignalColumn { column_index: usize },
    #[error("missing allocator weight stream for leg {leg_index}")]
    MissingAllocatorStream { leg_index: usize },
    #[error("OTL object compilation failed: {0}")]
    Compile(#[from] crate::orchestration::CompileError),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn column_major_layout_is_contiguous_per_series() {
        let block = ColumnMajorBlock::with_shape(4, 2);
        assert_eq!(block.flatten_primitive_array().len(), 8);
        let col0 = block.column_slice(0);
        let col1 = block.column_slice(1);
        assert_eq!(col0.len(), 4);
        assert_eq!(col1.len(), 4);
        assert!(
            col0.as_ptr() as usize + 4 * std::mem::size_of::<f64>()
                == col1.as_ptr() as usize,
            "columns must be adjacent in memory"
        );
    }

    #[test]
    fn graph_matrix_signal_column_mut_alias_flat_buffer() {
        let mut matrix = GraphSeriesMatrix::with_capacity(3, 2, 1);
        matrix.signal_column_slice_mut(1)[1] = 42.0;
        assert_eq!(matrix.read_signal(1, 1), Some(42.0));
    }

    #[test]
    fn column_primitive_array_matches_slice() {
        let block = ColumnMajorBlock::with_shape(3, 1);
        let array = block.column_primitive_array(0);
        assert_eq!(array.values(), &[0.0, 0.0, 0.0]);
    }
}
