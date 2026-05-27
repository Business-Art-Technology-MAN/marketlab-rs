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
        Ok(Self {
            master_len,
            stride_grid: MixedFrequencyStrideGrid::new(master_len)?,
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

    pub fn ingest_series(
        &mut self,
        spec: SeriesStrideSpec,
        native_values: &[f64],
    ) -> Result<(), ExecutionEngineError> {
        self.stride_grid.ingest_native_series(spec, native_values)
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
