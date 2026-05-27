//! Structural core for Layer 2 [`ExecutionEngine`] (SRD 2 execution surface).
//!
//! Responsibilities modeled here:
//! - **Continuous-time stage ledger** â€” cash/position samples on [`MarketStage`].
//! - **Mixed-frequency temporal stride arrays** â€” align heterogeneous cadences onto one master clock.
//! - **Causal forward-fill** â€” left-to-right carry with zero future look-ahead.
//! - **Topological graph compilation** â€” Kahn sort for deterministic node execution order.

mod stage_ledger;

use std::collections::{BTreeMap, VecDeque};
use std::fmt;

use ndarray::Array2;
use serde::{Deserialize, Serialize};

pub use stage_ledger::{
    position_prim_path, StageLedgerError, StageSimulationLedger, SimulationTransaction,
    EXECUTION_CASH_ATTR, EXECUTION_CASH_PATH, EXECUTION_POSITIONS_PREFIX,
};

// -----------------------------------------------------------------------------
// Errors
// -----------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum ExecutionEngineError {
    EmptyMasterTimeline,
    EmptyMatrixDims,
    MatrixShapeOverflow,
    MatrixShapePayloadMismatch { expected: usize, got: usize },
    MatrixConstructionFailed,
    InvalidStride { stride: usize },
    InvalidPhase { phase: usize, stride: usize },
    InvalidDuration { duration: f64 },
    InvalidPlayheadTime,
    SeriesLengthMismatch { series: String, expected: usize, got: usize },
    UnknownGraphNode(String),
    GraphCycleDetected,
    StageLedger(StageLedgerError),
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
            ExecutionEngineError::InvalidDuration { duration } => {
                write!(f, "duration must be finite and positive, got {duration}")
            }
            ExecutionEngineError::InvalidPlayheadTime => {
                write!(f, "playhead time must be finite")
            }
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
            ExecutionEngineError::StageLedger(error) => write!(f, "{error}"),
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
    pub time: f64,
    pub cash_delta: f64,
    pub position_deltas: Vec<(String, f64)>,
}

// -----------------------------------------------------------------------------
// Mixed-frequency temporal stride arrays (epoch duration windows)
// -----------------------------------------------------------------------------

pub const SECONDS_PER_DAY: f64 = 86_400.0;

/// One heterogeneous series aligned via rolling epoch-duration windows.
#[derive(Clone, Debug, PartialEq)]
pub struct SeriesDurationSpec {
    pub series_id: String,
    /// Rolling lookback width in seconds ending at the playhead.
    pub lookback_duration_secs: f64,
    /// Native cadence within the lookback window (e.g. 3600.0 for hourly).
    pub native_interval_secs: f64,
}

impl SeriesDurationSpec {
    pub fn validated(
        lookback_duration_secs: f64,
        native_interval_secs: f64,
    ) -> Result<(), ExecutionEngineError> {
        if !lookback_duration_secs.is_finite() || lookback_duration_secs <= 0.0 {
            return Err(ExecutionEngineError::InvalidDuration {
                duration: lookback_duration_secs,
            });
        }
        if !native_interval_secs.is_finite() || native_interval_secs <= 0.0 {
            return Err(ExecutionEngineError::InvalidDuration {
                duration: native_interval_secs,
            });
        }
        Ok(())
    }

    pub fn new(
        series_id: impl Into<String>,
        lookback_duration_secs: f64,
        native_interval_secs: f64,
    ) -> Result<Self, ExecutionEngineError> {
        Self::validated(lookback_duration_secs, native_interval_secs)?;
        Ok(Self {
            series_id: series_id.into(),
            lookback_duration_secs,
            native_interval_secs,
        })
    }

    /// Causal window `[playhead_time - lookback, playhead_time]`.
    pub fn window_bounds(&self, playhead_time: f64) -> Result<(f64, f64), ExecutionEngineError> {
        if !playhead_time.is_finite() {
            return Err(ExecutionEngineError::InvalidPlayheadTime);
        }
        Ok((playhead_time - self.lookback_duration_secs, playhead_time))
    }

    /// Deterministic resample timestamps inside the causal window.
    pub fn observation_times(&self, playhead_time: f64) -> Result<Vec<f64>, ExecutionEngineError> {
        let (start, end) = self.window_bounds(playhead_time)?;
        let mut times = Vec::new();
        let mut t = start;
        while t <= end + f64::EPSILON {
            times.push(t);
            t += self.native_interval_secs;
        }
        if times.last().map(|last| (*last - end).abs() > f64::EPSILON).unwrap_or(true) {
            times.push(end);
        }
        Ok(times)
    }
}

/// Legacy index stride spec retained for wire payloads; converts to epoch durations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SeriesStrideSpec {
    pub series_id: String,
    pub stride: usize,
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

    pub fn emission_indices(&self, master_len: usize) -> Vec<usize> {
        let mut indices = Vec::new();
        let mut t = self.phase;
        while t < master_len {
            indices.push(t);
            t = t.saturating_add(self.stride);
        }
        indices
    }

    pub fn to_duration_spec(&self, bar_duration_secs: f64) -> Result<SeriesDurationSpec, ExecutionEngineError> {
        SeriesDurationSpec::new(
            self.series_id.clone(),
            self.stride as f64 * bar_duration_secs,
            self.stride as f64 * bar_duration_secs,
        )
    }
}

/// Mixed-frequency stage query engine keyed by epoch-duration specs.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MixedFrequencyStrideGrid {
    series_index: BTreeMap<String, usize>,
    specs: Vec<SeriesDurationSpec>,
}

impl MixedFrequencyStrideGrid {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn series_count(&self) -> usize {
        self.specs.len()
    }

    pub fn spec(&self, series_id: &str) -> Option<&SeriesDurationSpec> {
        self.series_index
            .get(series_id)
            .map(|&row| &self.specs[row])
    }

    pub fn register_series(&mut self, spec: SeriesDurationSpec) -> Result<(), ExecutionEngineError> {
        if let Some(&row) = self.series_index.get(&spec.series_id) {
            self.specs[row] = spec;
        } else {
            let row = self.specs.len();
            self.series_index.insert(spec.series_id.clone(), row);
            self.specs.push(spec);
        }
        Ok(())
    }

    pub fn register_from_stride_spec(
        &mut self,
        spec: SeriesStrideSpec,
        bar_duration_secs: f64,
    ) -> Result<(), ExecutionEngineError> {
        self.register_series(spec.to_duration_spec(bar_duration_secs)?)
    }

    /// Query `MarketStage` over `(playhead_time - lookback) ..= playhead_time`.
    pub fn query_stage_window(
        &self,
        stage: &crate::trading_stage::MarketStage,
        series_id: &str,
        prim_path: &str,
        attribute: &str,
        playhead_time: f64,
    ) -> Result<Vec<f64>, ExecutionEngineError> {
        let spec = self
            .spec(series_id)
            .ok_or_else(|| ExecutionEngineError::UnknownGraphNode(series_id.to_string()))?;
        let (start, end) = spec.window_bounds(playhead_time)?;
        Ok(stage
            .samples_in_time_range(prim_path, attribute, start, end)
            .into_iter()
            .map(|(_, value)| f64::from(value))
            .collect())
    }

    /// Forward-fill stage values at the spec's observation timestamps.
    pub fn aligned_observations_at_playhead(
        &self,
        stage: &crate::trading_stage::MarketStage,
        series_id: &str,
        prim_path: &str,
        attribute: &str,
        playhead_time: f64,
    ) -> Result<Vec<f64>, ExecutionEngineError> {
        let spec = self
            .spec(series_id)
            .ok_or_else(|| ExecutionEngineError::UnknownGraphNode(series_id.to_string()))?;
        let times = spec.observation_times(playhead_time)?;
        Ok(times
            .into_iter()
            .filter_map(|t| stage.resolve_attribute_at(prim_path, attribute, t))
            .map(f64::from)
            .collect())
    }

    pub fn hydrate_from_wire(
        entries: &[SeriesStrideSpecWire],
        bar_duration_secs: f64,
    ) -> Result<Self, ExecutionEngineError> {
        let mut grid = Self::new();
        for entry in entries {
            let spec = SeriesStrideSpec::new(&entry.series_id, entry.stride, entry.phase)?;
            grid.register_from_stride_spec(spec, bar_duration_secs)?;
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
// ExecutionEngine orchestrator
// -----------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub struct ExecutionEngine {
    master_len: usize,
    stride_grid: MixedFrequencyStrideGrid,
    compiled_order: Vec<String>,
}

impl ExecutionEngine {
    pub fn bootstrap(master_len: usize) -> Result<Self, ExecutionEngineError> {
        if master_len == 0 {
            return Err(ExecutionEngineError::EmptyMasterTimeline);
        }
        Ok(Self {
            master_len,
            stride_grid: MixedFrequencyStrideGrid::new(),
            compiled_order: Vec::new(),
        })
    }

    pub fn master_len(&self) -> usize {
        self.master_len
    }

    pub fn stride_grid(&self) -> &MixedFrequencyStrideGrid {
        &self.stride_grid
    }

    pub fn stride_grid_mut(&mut self) -> &mut MixedFrequencyStrideGrid {
        &mut self.stride_grid
    }

    pub fn compiled_order(&self) -> &[String] {
        &self.compiled_order
    }

    pub fn compile_graph(&mut self, graph: &ExecutionGraph) -> Result<(), ExecutionEngineError> {
        self.compiled_order = graph.compile_execution_order()?;
        Ok(())
    }

    pub fn register_series_duration(
        &mut self,
        spec: SeriesDurationSpec,
    ) -> Result<(), ExecutionEngineError> {
        self.stride_grid.register_series(spec)
    }

    pub fn register_series_stride(
        &mut self,
        spec: SeriesStrideSpec,
        bar_duration_secs: f64,
    ) -> Result<(), ExecutionEngineError> {
        self.stride_grid
            .register_from_stride_spec(spec, bar_duration_secs)
    }

    pub fn query_series_window(
        &self,
        stage: &crate::trading_stage::MarketStage,
        series_id: &str,
        prim_path: &str,
        attribute: &str,
        playhead_time: f64,
    ) -> Result<Vec<f64>, ExecutionEngineError> {
        self.stride_grid
            .query_stage_window(stage, series_id, prim_path, attribute, playhead_time)
    }

    pub fn ingest_series(
        &mut self,
        spec: SeriesStrideSpec,
        _native_values: &[f64],
    ) -> Result<(), ExecutionEngineError> {
        self.register_series_stride(spec, SECONDS_PER_DAY)
    }

    pub fn apply_transaction(
        stage: &mut crate::trading_stage::MarketStage,
        tx: &SimulationTransaction,
    ) -> Result<(), ExecutionEngineError> {
        StageSimulationLedger::apply_transaction(stage, tx)
            .map_err(ExecutionEngineError::StageLedger)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trading_stage::MarketStage;

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
    fn duration_spec_window_bounds_are_causal() {
        let spec = SeriesDurationSpec::new("daily", SECONDS_PER_DAY * 5.0, SECONDS_PER_DAY).unwrap();
        let (start, end) = spec.window_bounds(1_000_000.0).unwrap();
        assert_eq!(end - start, SECONDS_PER_DAY * 5.0);
    }

    #[test]
    fn mixed_frequency_grid_queries_stage_window() {
        let mut grid = MixedFrequencyStrideGrid::new();
        grid.register_series(
            SeriesDurationSpec::new(
                "daily_close",
                SECONDS_PER_DAY * 3.0,
                SECONDS_PER_DAY,
            )
            .unwrap(),
        )
        .unwrap();
        let mut stage = MarketStage::new();
        let prim = crate::trading_stage::asset_prim_path("SPY").unwrap();
        stage.set_sample(&prim, "close", 100.0, 10.0).unwrap();
        stage.set_sample(&prim, "close", 100.0 + SECONDS_PER_DAY, 20.0).unwrap();
        stage.set_sample(&prim, "close", 100.0 + SECONDS_PER_DAY * 2.0, 30.0).unwrap();
        let playhead = 100.0 + SECONDS_PER_DAY * 2.0;
        let window = grid
            .query_stage_window(&stage, "daily_close", &prim, "close", playhead)
            .unwrap();
        assert_eq!(window, vec![10.0, 20.0, 30.0]);
    }

    #[test]
    fn aligned_observations_forward_fill_from_stage() {
        let mut grid = MixedFrequencyStrideGrid::new();
        grid.register_series(
            SeriesDurationSpec::new(
                "daily_close",
                SECONDS_PER_DAY * 2.0,
                SECONDS_PER_DAY,
            )
            .unwrap(),
        )
        .unwrap();
        let mut stage = MarketStage::new();
        let prim = crate::trading_stage::asset_prim_path("SPY").unwrap();
        stage.set_sample(&prim, "close", 100.0, 10.0).unwrap();
        stage.set_sample(&prim, "close", 100.0 + SECONDS_PER_DAY, 20.0).unwrap();
        let playhead = 100.0 + SECONDS_PER_DAY + SECONDS_PER_DAY / 2.0;
        let values = grid
            .aligned_observations_at_playhead(&stage, "daily_close", &prim, "close", playhead)
            .unwrap();
        assert_eq!(values, vec![10.0, 20.0]);
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
    fn engine_bootstraps_with_master_len() {
        let engine = ExecutionEngine::bootstrap(16).unwrap();
        assert_eq!(engine.master_len(), 16);
    }

    #[test]
    fn stage_transaction_updates_ledger() {
        let mut stage = MarketStage::new();
        StageSimulationLedger::seed_initial_cash(&mut stage, 1_000.0).unwrap();
        ExecutionEngine::apply_transaction(
            &mut stage,
            &SimulationTransaction {
                time: 10.0,
                cash_delta: -100.0,
                position_deltas: vec![("SPY".into(), 2.0)],
            },
        )
        .unwrap();
        assert_eq!(StageSimulationLedger::cash_at(&stage, 10.0), 900.0);
        assert_eq!(StageSimulationLedger::shares_at(&stage, "SPY", 10.0), 2.0);
    }
}
