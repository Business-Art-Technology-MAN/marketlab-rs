//! Structural core for Layer 2 [`ExecutionEngine`] (SRD 2 execution surface).
//!
//! Responsibilities modeled here:
//! - **Core tracking matrix** — dense per-tick execution channel ledger (`ndarray::Array2`).
//! - **Mixed-frequency temporal stride arrays** — align heterogeneous cadences onto one master clock.
//! - **Causal forward-fill** — left-to-right carry with zero future look-ahead.
//! - **Topological graph compilation** — Kahn sort for deterministic node execution order.
//! - **Simulation account matrices** — cash pool + asset quantity pools per master tick.

use std::collections::{BTreeMap, VecDeque};
use std::fmt;

use ndarray::Array2;
use serde::{Deserialize, Serialize};

use crate::trading_stage::TradingStage;

// -----------------------------------------------------------------------------
// Layer 1 abstraction
// -----------------------------------------------------------------------------

/// Read-only surface Layer 1 exposes to the execution engine.
pub trait TradingStageFeed {
    fn stage_matrix(&self) -> &Array2<f64>;
    fn master_timeline_len(&self) -> usize;
}

impl TradingStageFeed for TradingStage {
    fn stage_matrix(&self) -> &Array2<f64> {
        self.matrix()
    }

    fn master_timeline_len(&self) -> usize {
        self.matrix().nrows()
    }
}

// -----------------------------------------------------------------------------
// Errors
// -----------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutionEngineError {
    EmptyMasterTimeline,
    EmptyMatrixDims,
    MatrixShapeOverflow,
    MatrixShapePayloadMismatch { expected: usize, got: usize },
    MatrixConstructionFailed,
    InvalidStride { stride: usize },
    InvalidPhase { phase: usize, stride: usize },
    SeriesLengthMismatch { series: String, expected: usize, got: usize },
    UnknownGraphNode(String),
    GraphCycleDetected,
    AccountColumnOutOfRange { col: usize, width: usize },
    TransactionTimeOutOfRange { t: usize, rows: usize },
}

impl fmt::Display for ExecutionEngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExecutionEngineError::EmptyMasterTimeline => {
                write!(f, "master timeline must contain at least one tick")
            }
            ExecutionEngineError::EmptyMatrixDims => {
                write!(f, "matrix rows/cols must be positive")
            }
            ExecutionEngineError::MatrixShapeOverflow => {
                write!(f, "matrix row * col overflowed usize")
            }
            ExecutionEngineError::MatrixShapePayloadMismatch { expected, got } => write!(
                f,
                "matrix row_major_flat length {got} does not match rows*cols {expected}"
            ),
            ExecutionEngineError::MatrixConstructionFailed => {
                write!(f, "ndarray could not assume row-major shape")
            }
            ExecutionEngineError::InvalidStride { stride } => {
                write!(f, "stride must be >= 1, got {stride}")
            }
            ExecutionEngineError::InvalidPhase { phase, stride } => write!(
                f,
                "phase {phase} must be strictly less than stride {stride}"
            ),
            ExecutionEngineError::SeriesLengthMismatch {
                series,
                expected,
                got,
            } => write!(
                f,
                "series `{series}` native payload length {got} != emission count {expected}"
            ),
            ExecutionEngineError::UnknownGraphNode(id) => {
                write!(f, "graph edge references unknown node `{id}`")
            }
            ExecutionEngineError::GraphCycleDetected => {
                write!(f, "execution graph contains a cycle")
            }
            ExecutionEngineError::AccountColumnOutOfRange { col, width } => write!(
                f,
                "account column {col} out of range for width {width}"
            ),
            ExecutionEngineError::TransactionTimeOutOfRange { t, rows } => write!(
                f,
                "transaction time index {t} out of range for {rows} rows"
            ),
        }
    }
}

impl std::error::Error for ExecutionEngineError {}

// -----------------------------------------------------------------------------
// Wire payloads
// -----------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionMatrixWire {
    pub rows: usize,
    pub cols: usize,
    pub row_major_flat: Vec<f64>,
}

impl ExecutionMatrixWire {
    pub fn validated_array(self) -> Result<Array2<f64>, ExecutionEngineError> {
        let ExecutionMatrixWire {
            rows,
            cols,
            row_major_flat,
        } = self;
        if rows == 0 || cols == 0 {
            return Err(ExecutionEngineError::EmptyMatrixDims);
        }
        let cells = rows
            .checked_mul(cols)
            .ok_or(ExecutionEngineError::MatrixShapeOverflow)?;
        if row_major_flat.len() != cells {
            return Err(ExecutionEngineError::MatrixShapePayloadMismatch {
                expected: cells,
                got: row_major_flat.len(),
            });
        }
        Array2::from_shape_vec((rows, cols), row_major_flat)
            .map_err(|_| ExecutionEngineError::MatrixConstructionFailed)
    }

    pub fn try_from_array(array: Array2<f64>) -> Result<Self, ExecutionEngineError> {
        let shape = array.raw_dim();
        let flat: Vec<f64> = array.iter().copied().collect();
        Ok(ExecutionMatrixWire {
            rows: shape[0],
            cols: shape[1],
            row_major_flat: flat,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeriesStrideSpecWire {
    pub series_id: String,
    pub stride: usize,
    pub phase: usize,
    pub native_values: Vec<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionGraphWire {
    pub nodes: Vec<String>,
    pub edges: Vec<(String, String)>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SimulationTransactionWire {
    pub time_index: usize,
    pub cash_delta: f64,
    pub asset_deltas: Vec<(usize, f64)>,
}

// -----------------------------------------------------------------------------
// Mixed-frequency temporal stride arrays
// -----------------------------------------------------------------------------

/// One heterogeneous series aligned onto the master clock via `(stride, phase)`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SeriesStrideSpec {
    pub series_id: String,
    /// Master ticks between native observations (must be >= 1).
    pub stride: usize,
    /// First native emission lands at master index `phase` (must be < stride).
    pub phase: usize,
}

impl SeriesStrideSpec {
    pub fn validated(stride: usize, phase: usize) -> Result<(usize, usize), ExecutionEngineError> {
        if stride == 0 {
            return Err(ExecutionEngineError::InvalidStride { stride });
        }
        if phase >= stride {
            return Err(ExecutionEngineError::InvalidPhase { phase, stride });
        }
        Ok((stride, phase))
    }

    pub fn new(
        series_id: impl Into<String>,
        stride: usize,
        phase: usize,
    ) -> Result<Self, ExecutionEngineError> {
        Self::validated(stride, phase)?;
        Ok(Self {
            series_id: series_id.into(),
            stride,
            phase,
        })
    }

    /// Deterministic emission indices on the master timeline (inclusive start, exclusive end).
    pub fn emission_indices(&self, master_len: usize) -> Vec<usize> {
        let mut indices = Vec::new();
        let mut t = self.phase;
        while t < master_len {
            indices.push(t);
            t = t.saturating_add(self.stride);
        }
        indices
    }
}

/// Dense per-series rows forward-filled onto the master clock.
#[derive(Clone, Debug, PartialEq)]
pub struct MixedFrequencyStrideGrid {
    master_len: usize,
    /// Row-major: one row per registered series, `cols == master_len`.
    aligned: Array2<f64>,
    series_index: BTreeMap<String, usize>,
    specs: Vec<SeriesStrideSpec>,
}

impl MixedFrequencyStrideGrid {
    pub fn new(master_len: usize) -> Result<Self, ExecutionEngineError> {
        if master_len == 0 {
            return Err(ExecutionEngineError::EmptyMasterTimeline);
        }
        Ok(Self {
            master_len,
            aligned: Array2::zeros((0, master_len)),
            series_index: BTreeMap::new(),
            specs: Vec::new(),
        })
    }

    pub fn master_len(&self) -> usize {
        self.master_len
    }

    pub fn series_count(&self) -> usize {
        self.specs.len()
    }

    pub fn spec(&self, series_id: &str) -> Option<&SeriesStrideSpec> {
        self.series_index
            .get(series_id)
            .map(|&row| &self.specs[row])
    }

    pub fn aligned_row(&self, series_id: &str) -> Option<ndarray::ArrayView1<'_, f64>> {
        self.series_index
            .get(series_id)
            .map(|&row| self.aligned.row(row))
    }

    pub fn aligned(&self) -> &Array2<f64> {
        &self.aligned
    }

    /// Register a native-frequency payload and materialize its causal forward-filled row.
    pub fn ingest_native_series(
        &mut self,
        spec: SeriesStrideSpec,
        native_values: &[f64],
    ) -> Result<(), ExecutionEngineError> {
        if self.series_index.contains_key(&spec.series_id) {
            self.specs[self.series_index[&spec.series_id]] = spec.clone();
        } else {
            let row = self.specs.len();
            self.series_index.insert(spec.series_id.clone(), row);
            self.specs.push(spec.clone());
            if self.aligned.nrows() == 0 {
                self.aligned = Array2::zeros((1, self.master_len));
            } else {
                self.aligned
                    .append(
                        ndarray::Axis(0),
                        Array2::zeros((1, self.master_len)).view(),
                    )
                    .expect("append single row");
            }
        }

        let emissions = spec.emission_indices(self.master_len);
        if native_values.len() != emissions.len() {
            return Err(ExecutionEngineError::SeriesLengthMismatch {
                series: spec.series_id.clone(),
                expected: emissions.len(),
                got: native_values.len(),
            });
        }

        let row_idx = self.series_index[&spec.series_id];
        let mut scratch = vec![f64::NAN; self.master_len];
        forward_fill_causal(&mut scratch, &emissions, native_values);
        for (t, value) in scratch.iter().enumerate() {
            self.aligned[(row_idx, t)] = *value;
        }
        Ok(())
    }

    pub fn hydrate_from_wire(
        master_len: usize,
        entries: &[SeriesStrideSpecWire],
    ) -> Result<Self, ExecutionEngineError> {
        let mut grid = Self::new(master_len)?;
        for entry in entries {
            let spec = SeriesStrideSpec::new(&entry.series_id, entry.stride, entry.phase)?;
            grid.ingest_native_series(spec, &entry.native_values)?;
        }
        Ok(grid)
    }
}

// -----------------------------------------------------------------------------
// Causal forward-fill (zero future look-ahead)
// -----------------------------------------------------------------------------

/// Left-to-right carry: at master index `t` only observations with index `<= t` are visible.
pub fn forward_fill_causal(
    out: &mut [f64],
    native_indices: &[usize],
    native_values: &[f64],
) {
    debug_assert_eq!(native_indices.len(), native_values.len());
    let mut obs = 0usize;
    let mut last = f64::NAN;
    for (t, slot) in out.iter_mut().enumerate() {
        while obs < native_indices.len() && native_indices[obs] <= t {
            last = native_values[obs];
            obs += 1;
        }
        *slot = last;
    }
}

/// Returns `true` when `out[t]` never reads an observation whose native index is `> t`.
pub fn forward_fill_is_causal(
    out: &[f64],
    native_indices: &[usize],
    native_values: &[f64],
) -> bool {
    if native_indices.len() != native_values.len() {
        return false;
    }
    let mut probe = vec![f64::NAN; out.len()];
    forward_fill_causal(&mut probe, native_indices, native_values);
    for t in 0..out.len() {
        let mut obs = 0usize;
        let mut last = f64::NAN;
        while obs < native_indices.len() && native_indices[obs] <= t {
            last = native_values[obs];
            obs += 1;
        }
        if probe[t] != last {
            return false;
        }
    }
    true
}

// -----------------------------------------------------------------------------
// Topological graph compilation (Kahn's algorithm)
// -----------------------------------------------------------------------------

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExecutionGraph {
    nodes: BTreeMap<String, Vec<String>>,
}

impl ExecutionGraph {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_node(&mut self, id: impl Into<String>) {
        self.nodes.entry(id.into()).or_default();
    }

    pub fn add_edge(&mut self, from: impl Into<String>, to: impl Into<String>) {
        let from = from.into();
        let to = to.into();
        self.nodes.entry(from.clone()).or_default().push(to.clone());
        self.nodes.entry(to).or_default();
    }

    pub fn node_ids(&self) -> impl Iterator<Item = &String> {
        self.nodes.keys()
    }

    /// Deterministic topological order (`BTreeMap` seeding + stable queue drain).
    pub fn compile_execution_order(&self) -> Result<Vec<String>, ExecutionEngineError> {
        topological_sort_kahn(&self.nodes)
    }

    pub fn from_wire(wire: &ExecutionGraphWire) -> Result<Self, ExecutionEngineError> {
        let mut graph = Self::new();
        for node in &wire.nodes {
            graph.add_node(node.clone());
        }
        for (from, to) in &wire.edges {
            if !graph.nodes.contains_key(from) {
                return Err(ExecutionEngineError::UnknownGraphNode(from.clone()));
            }
            if !graph.nodes.contains_key(to) {
                return Err(ExecutionEngineError::UnknownGraphNode(to.clone()));
            }
            graph.add_edge(from.clone(), to.clone());
        }
        Ok(graph)
    }
}

pub fn topological_sort_kahn(
    adjacency: &BTreeMap<String, Vec<String>>,
) -> Result<Vec<String>, ExecutionEngineError> {
    let mut indegree: BTreeMap<String, usize> = adjacency
        .keys()
        .map(|id| (id.clone(), 0usize))
        .collect();

    for edges in adjacency.values() {
        for to in edges {
            if !adjacency.contains_key(to) {
                return Err(ExecutionEngineError::UnknownGraphNode(to.clone()));
            }
            *indegree.get_mut(to).expect("seeded") += 1;
        }
    }

    let mut ready: VecDeque<String> = indegree
        .iter()
        .filter(|(_, &deg)| deg == 0)
        .map(|(id, _)| id.clone())
        .collect();

    let mut order = Vec::with_capacity(indegree.len());
    while let Some(node) = ready.pop_front() {
        order.push(node.clone());
        if let Some(edges) = adjacency.get(&node) {
            for to in edges {
                let entry = indegree.get_mut(to).expect("seeded");
                *entry = entry.saturating_sub(1);
                if *entry == 0 {
                    ready.push_back(to.clone());
                }
            }
        }
    }

    if order.len() != indegree.len() {
        return Err(ExecutionEngineError::GraphCycleDetected);
    }
    Ok(order)
}

// -----------------------------------------------------------------------------
// Simulation transaction account matrices (cash + asset pools)
// -----------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountColumn {
    Cash,
    Asset(usize),
}

impl AccountColumn {
    pub fn index(self) -> usize {
        match self {
            AccountColumn::Cash => 0,
            AccountColumn::Asset(i) => i.saturating_add(1),
        }
    }
}

/// Per-tick ledger: column 0 is cash; columns `1..=` are asset quantity pools.
#[derive(Clone, Debug, PartialEq)]
pub struct SimulationAccountMatrix {
    /// Row 0 seeds initial balances; row `t>0` derives from row `t-1` + transactions at `t`.
    state: Array2<f64>,
    asset_pool_count: usize,
}

impl SimulationAccountMatrix {
    pub fn new(
        master_len: usize,
        asset_pool_count: usize,
        initial_cash: f64,
        initial_asset_qty: &[f64],
    ) -> Result<Self, ExecutionEngineError> {
        if master_len == 0 {
            return Err(ExecutionEngineError::EmptyMasterTimeline);
        }
        if initial_asset_qty.len() != asset_pool_count {
            return Err(ExecutionEngineError::SeriesLengthMismatch {
                series: "initial_asset_qty".into(),
                expected: asset_pool_count,
                got: initial_asset_qty.len(),
            });
        }
        let cols = asset_pool_count.saturating_add(1);
        let mut state = Array2::zeros((master_len, cols));
        state[(0, AccountColumn::Cash.index())] = initial_cash;
        for (i, qty) in initial_asset_qty.iter().enumerate() {
            state[(0, AccountColumn::Asset(i).index())] = *qty;
        }
        let mut account = Self {
            state,
            asset_pool_count,
        };
        // Carry initial balances forward so reads at t>0 are valid before any transaction.
        account.propagate_tail_from(0);
        Ok(account)
    }

    pub fn rows(&self) -> usize {
        self.state.nrows()
    }

    pub fn cols(&self) -> usize {
        self.state.ncols()
    }

    pub fn asset_pool_count(&self) -> usize {
        self.asset_pool_count
    }

    pub fn state(&self) -> &Array2<f64> {
        &self.state
    }

    pub fn cash_at(&self, t: usize) -> Result<f64, ExecutionEngineError> {
        self.read_cell(t, AccountColumn::Cash)
    }

    pub fn asset_qty_at(&self, t: usize, pool: usize) -> Result<f64, ExecutionEngineError> {
        if pool >= self.asset_pool_count {
            return Err(ExecutionEngineError::AccountColumnOutOfRange {
                col: AccountColumn::Asset(pool).index(),
                width: self.cols(),
            });
        }
        self.read_cell(t, AccountColumn::Asset(pool))
    }

    fn read_cell(&self, t: usize, col: AccountColumn) -> Result<f64, ExecutionEngineError> {
        if t >= self.rows() {
            return Err(ExecutionEngineError::TransactionTimeOutOfRange {
                t,
                rows: self.rows(),
            });
        }
        Ok(self.state[(t, col.index())])
    }

    pub fn apply_transaction(
        &mut self,
        tx: &SimulationTransaction,
    ) -> Result<(), ExecutionEngineError> {
        if tx.time_index >= self.rows() {
            return Err(ExecutionEngineError::TransactionTimeOutOfRange {
                t: tx.time_index,
                rows: self.rows(),
            });
        }
        let t = tx.time_index;
        if t > 0 {
            for c in 0..self.cols() {
                self.state[(t, c)] = self.state[(t - 1, c)];
            }
        }
        self.state[(t, AccountColumn::Cash.index())] += tx.cash_delta;
        for &(pool, delta) in &tx.asset_deltas {
            let col = AccountColumn::Asset(pool).index();
            if col >= self.cols() {
                return Err(ExecutionEngineError::AccountColumnOutOfRange {
                    col,
                    width: self.cols(),
                });
            }
            self.state[(t, col)] += delta;
        }
        self.propagate_tail_from(t);
        Ok(())
    }

    fn propagate_tail_from(&mut self, start: usize) {
        for t in (start + 1)..self.rows() {
            for c in 0..self.cols() {
                self.state[(t, c)] = self.state[(t - 1, c)];
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SimulationTransaction {
    pub time_index: usize,
    pub cash_delta: f64,
    pub asset_deltas: Vec<(usize, f64)>,
}

// -----------------------------------------------------------------------------
// Core execution tracking matrix
// -----------------------------------------------------------------------------

/// Dense per-tick channel ledger (rows = master timeline, cols = execution channels).
#[derive(Clone, Debug, PartialEq)]
pub struct ExecutionTrackingMatrix {
    channels: BTreeMap<String, usize>,
    values: Array2<f64>,
}

impl ExecutionTrackingMatrix {
    pub fn new(master_len: usize, channel_ids: &[String]) -> Result<Self, ExecutionEngineError> {
        if master_len == 0 {
            return Err(ExecutionEngineError::EmptyMasterTimeline);
        }
        let mut channels = BTreeMap::new();
        for (idx, id) in channel_ids.iter().enumerate() {
            channels.insert(id.clone(), idx);
        }
        Ok(Self {
            channels,
            values: Array2::zeros((master_len, channel_ids.len().max(1))),
        })
    }

    pub fn from_channel_map(
        master_len: usize,
        channels: BTreeMap<String, usize>,
    ) -> Result<Self, ExecutionEngineError> {
        if master_len == 0 {
            return Err(ExecutionEngineError::EmptyMasterTimeline);
        }
        let width = channels.values().copied().max().map(|m| m + 1).unwrap_or(1);
        Ok(Self {
            channels,
            values: Array2::zeros((master_len, width)),
        })
    }

    pub fn rows(&self) -> usize {
        self.values.nrows()
    }

    pub fn cols(&self) -> usize {
        self.values.ncols()
    }

    pub fn channels(&self) -> &BTreeMap<String, usize> {
        &self.channels
    }

    pub fn values(&self) -> &Array2<f64> {
        &self.values
    }

    pub fn values_mut(&mut self) -> &mut Array2<f64> {
        &mut self.values
    }

    pub fn ensure_channel(&mut self, channel: &str) -> Result<usize, ExecutionEngineError> {
        if let Some(col) = self.channels.get(channel) {
            return Ok(*col);
        }
        let col = self.cols();
        self.channels.insert(channel.to_string(), col);
        if col == 0 && self.values.ncols() == 0 {
            self.values = Array2::zeros((self.rows(), 1));
        } else {
            let (rows, old_cols) = self.values.dim();
            let mut expanded = Array2::zeros((rows, old_cols + 1));
            if old_cols > 0 {
                expanded.slice_mut(ndarray::s![.., ..old_cols]).assign(&self.values);
            }
            self.values = expanded;
        }
        Ok(col)
    }

    pub fn set_channel(&mut self, t: usize, channel: &str, value: f64) -> Result<(), ExecutionEngineError> {
        self.ensure_channel(channel)?;
        let col = self
            .channels
            .get(channel)
            .copied()
            .ok_or_else(|| ExecutionEngineError::UnknownGraphNode(channel.to_string()))?;
        if t >= self.rows() {
            return Err(ExecutionEngineError::TransactionTimeOutOfRange {
                t,
                rows: self.rows(),
            });
        }
        self.values[(t, col)] = value;
        Ok(())
    }

    pub fn channel_at(&self, t: usize, channel: &str) -> Result<f64, ExecutionEngineError> {
        let col = self
            .channels
            .get(channel)
            .copied()
            .ok_or_else(|| ExecutionEngineError::UnknownGraphNode(channel.to_string()))?;
        if t >= self.rows() {
            return Err(ExecutionEngineError::TransactionTimeOutOfRange {
                t,
                rows: self.rows(),
            });
        }
        Ok(self.values[(t, col)])
    }

    /// Zero every sample in `channel` (used when a downstream wire is disconnected).
    pub fn clear_channel_data(&mut self, channel: &str) -> Result<(), ExecutionEngineError> {
        let col = self
            .channels
            .get(channel)
            .copied()
            .ok_or_else(|| ExecutionEngineError::UnknownGraphNode(channel.to_string()))?;
        for t in 0..self.rows() {
            self.values[(t, col)] = 0.0;
        }
        Ok(())
    }

    /// Walk backward from `t` until a finite, non-zero sample is found for `channel`.
    pub fn resolve_channel_at(&self, t: usize, channel: &str) -> Result<f64, ExecutionEngineError> {
        if t >= self.rows() {
            return Err(ExecutionEngineError::TransactionTimeOutOfRange {
                t,
                rows: self.rows(),
            });
        }
        for index in (0..=t).rev() {
            let sample = self.channel_at(index, channel)?;
            if sample.is_finite() && sample > 0.0 {
                return Ok(sample);
            }
        }
        Ok(0.0)
    }

    pub fn bind_stride_grid(&mut self, grid: &MixedFrequencyStrideGrid, channel_prefix: &str) {
        for spec in &grid.specs {
            if let Some(row) = grid.aligned_row(&spec.series_id) {
                if let Some(&col) = self.channels.get(&format!("{channel_prefix}{}", spec.series_id)) {
                    for t in 0..self.rows().min(row.len()) {
                        self.values[(t, col)] = row[t];
                    }
                }
            }
        }
    }
}

// -----------------------------------------------------------------------------
// ExecutionEngine orchestrator
// -----------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub struct ExecutionEngine {
    master_len: usize,
    tracking: ExecutionTrackingMatrix,
    stride_grid: MixedFrequencyStrideGrid,
    account: SimulationAccountMatrix,
    compiled_order: Vec<String>,
}

impl ExecutionEngine {
    pub fn bootstrap(
        stage: &impl TradingStageFeed,
        channel_ids: &[String],
        asset_pool_count: usize,
        initial_cash: f64,
        initial_asset_qty: &[f64],
    ) -> Result<Self, ExecutionEngineError> {
        let master_len = stage.master_timeline_len();
        Ok(Self {
            master_len,
            tracking: ExecutionTrackingMatrix::new(master_len, channel_ids)?,
            stride_grid: MixedFrequencyStrideGrid::new(master_len)?,
            account: SimulationAccountMatrix::new(
                master_len,
                asset_pool_count,
                initial_cash,
                initial_asset_qty,
            )?,
            compiled_order: Vec::new(),
        })
    }

    pub fn master_len(&self) -> usize {
        self.master_len
    }

    pub fn tracking(&self) -> &ExecutionTrackingMatrix {
        &self.tracking
    }

    pub fn tracking_mut(&mut self) -> &mut ExecutionTrackingMatrix {
        &mut self.tracking
    }

    pub fn stride_grid(&self) -> &MixedFrequencyStrideGrid {
        &self.stride_grid
    }

    pub fn stride_grid_mut(&mut self) -> &mut MixedFrequencyStrideGrid {
        &mut self.stride_grid
    }

    pub fn account(&self) -> &SimulationAccountMatrix {
        &self.account
    }

    pub fn account_mut(&mut self) -> &mut SimulationAccountMatrix {
        &mut self.account
    }

    pub fn compiled_order(&self) -> &[String] {
        &self.compiled_order
    }

    pub fn compile_graph(&mut self, graph: &ExecutionGraph) -> Result<(), ExecutionEngineError> {
        self.compiled_order = graph.compile_execution_order()?;
        Ok(())
    }

    pub fn ingest_series(
        &mut self,
        spec: SeriesStrideSpec,
        native_values: &[f64],
    ) -> Result<(), ExecutionEngineError> {
        self.stride_grid.ingest_native_series(spec, native_values)
    }

    pub fn apply_transaction(&mut self, tx: &SimulationTransaction) -> Result<(), ExecutionEngineError> {
        self.account.apply_transaction(tx)
    }

    pub fn ensure_tracking_channel(&mut self, channel: &str) -> Result<usize, ExecutionEngineError> {
        self.tracking.ensure_channel(channel)
    }

    pub fn write_tracking_sample(
        &mut self,
        t: usize,
        channel: &str,
        value: f64,
    ) -> Result<(), ExecutionEngineError> {
        self.tracking.set_channel(t, channel, value)
    }

    pub fn clear_channel_data(&mut self, channel: &str) -> Result<(), ExecutionEngineError> {
        self.tracking.clear_channel_data(channel)
    }

    /// Resolve `market.raw` at tick `t`, walking backward when forward-fill yields zero.
    pub fn resolve_mark_price_at(&self, t: usize) -> f64 {
        self.tracking
            .resolve_channel_at(t, "market.raw")
            .unwrap_or(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trading_stage::TradingStage;
    use ndarray::Array2;

    #[test]
    fn forward_fill_never_peeks_future() {
        let native_indices = vec![0, 5, 10];
        let native_values = vec![1.0, 2.0, 3.0];
        let mut out = vec![0.0; 12];
        forward_fill_causal(&mut out, &native_indices, &native_values);
        assert!(forward_fill_is_causal(&out, &native_indices, &native_values));
        assert_eq!(out[4], 1.0);
        assert_eq!(out[5], 2.0);
        assert_eq!(out[11], 3.0);
    }

    #[test]
    fn stride_emissions_are_phase_offset() {
        let spec = SeriesStrideSpec::new("daily", 4, 1).unwrap();
        assert_eq!(spec.emission_indices(10), vec![1, 5, 9]);
    }

    #[test]
    fn mixed_frequency_grid_aligns_rows() {
        let mut grid = MixedFrequencyStrideGrid::new(8).unwrap();
        let spec = SeriesStrideSpec::new("h1", 2, 0).unwrap();
        grid.ingest_native_series(spec, &[10.0, 20.0, 30.0, 40.0])
            .unwrap();
        let row = grid.aligned_row("h1").unwrap();
        assert_eq!(row[0], 10.0);
        assert_eq!(row[1], 10.0);
        assert_eq!(row[2], 20.0);
    }

    #[test]
    fn topological_sort_detects_cycle() {
        let mut adj = BTreeMap::new();
        adj.insert("a".into(), vec!["b".into()]);
        adj.insert("b".into(), vec!["c".into()]);
        adj.insert("c".into(), vec!["a".into()]);
        assert_eq!(
            topological_sort_kahn(&adj),
            Err(ExecutionEngineError::GraphCycleDetected)
        );
    }

    #[test]
    fn tracking_matrix_grows_channels_on_demand() {
        let mut tracking = ExecutionTrackingMatrix::new(4, &["seed".into()]).unwrap();
        tracking.ensure_channel("ta.2.value").unwrap();
        tracking.set_channel(1, "ta.2.value", 42.0).unwrap();
        assert_eq!(tracking.channel_at(1, "ta.2.value").unwrap(), 42.0);
        assert_eq!(tracking.cols(), 2);
    }

    #[test]
    fn account_initial_balances_propagate_to_all_ticks() {
        let acct = SimulationAccountMatrix::new(32, 1, 10_000.0, &[0.0]).unwrap();
        assert_eq!(acct.cash_at(0).unwrap(), 10_000.0);
        assert_eq!(acct.cash_at(21).unwrap(), 10_000.0);
        assert_eq!(acct.asset_qty_at(21, 0).unwrap(), 0.0);
    }

    #[test]
    fn resolve_channel_at_walks_backward() {
        let mut tracking = ExecutionTrackingMatrix::new(8, &["market.raw".into()]).unwrap();
        tracking.set_channel(2, "market.raw", 101.0).unwrap();
        assert_eq!(tracking.resolve_channel_at(5, "market.raw").unwrap(), 101.0);
    }

    #[test]
    fn clear_channel_data_zeros_samples() {
        let mut tracking = ExecutionTrackingMatrix::new(4, &["ta.1.value".into()]).unwrap();
        tracking.set_channel(1, "ta.1.value", 55.0).unwrap();
        tracking.clear_channel_data("ta.1.value").unwrap();
        assert_eq!(tracking.channel_at(1, "ta.1.value").unwrap(), 0.0);
    }

    #[test]
    fn account_matrix_tracks_cash_and_assets() {
        let mut acct = SimulationAccountMatrix::new(4, 1, 1000.0, &[0.0]).unwrap();
        acct.apply_transaction(&SimulationTransaction {
            time_index: 1,
            cash_delta: -50.0,
            asset_deltas: vec![(0, 2.0)],
        })
        .unwrap();
        assert_eq!(acct.cash_at(1).unwrap(), 950.0);
        assert_eq!(acct.asset_qty_at(1, 0).unwrap(), 2.0);
        assert_eq!(acct.cash_at(3).unwrap(), 950.0);
    }

    #[test]
    fn engine_bootstraps_from_trading_stage() {
        let stage = TradingStage::new(Array2::zeros((16, 3)));
        let channels = vec!["signal.a".into(), "signal.b".into()];
        let engine = ExecutionEngine::bootstrap(&stage, &channels, 2, 10_000.0, &[0.0, 0.0]).unwrap();
        assert_eq!(engine.master_len(), 16);
        assert_eq!(engine.tracking().cols(), 2);
    }
}
