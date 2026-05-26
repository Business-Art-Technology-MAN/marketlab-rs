//! Structural core for Layer 1 [`TradingStage`] (SRD 1 path surface).
//!
//! Responsibilities modeled here:
//! - Primary numeric **matrix** for dense stage values (`ndarray::Array2`).
//! - **Path-addressable primitive maps**, one logical bucket per stacking tier (`Base`, `Signals`, `Overrides`).
//! - Deterministic **`DataStackLayer` pipeline ordering** (`Base → Signals → Overrides` for merges).
//! - **Dirty collectors** listing paths/cells mutated since last drain.

use std::collections::BTreeMap;
use std::fmt;

use ndarray::Array2;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Stacking tiers applied in succession; later tiers override lookups from earlier ones.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataStackLayer {
    Base,
    Signals,
    Overrides,
}

impl DataStackLayer {
    /// Canonical merge-read order from foundation toward overlay overrides.
    pub const PIPELINE: [DataStackLayer; 3] = [Self::Base, Self::Signals, Self::Overrides];
}

impl fmt::Display for DataStackLayer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            DataStackLayer::Base => "base",
            DataStackLayer::Signals => "signals",
            DataStackLayer::Overrides => "overrides",
        };
        f.write_str(s)
    }
}

/// Discrete values stored behind dot-path keys (serialized map keys are plain strings).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Primitive {
    Bool(bool),
    I64(i64),
    F64(f64),
    Text(String),
}

/// Canonical wire shape for serialized maps of [`Primitive`] (stable key order via `BTreeMap`).
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PathPrimitiveWire {
    pub entries: BTreeMap<String, Primitive>,
}

fn validate_path_segments(path: &str) -> Result<(), TierPathError> {
    if path.is_empty() || path.starts_with('.') || path.ends_with('.') || path.contains("..") {
        return Err(TierPathError::InvalidPath);
    }
    for seg in path.split('.') {
        if seg.is_empty() {
            return Err(TierPathError::InvalidPath);
        }
    }
    Ok(())
}

/// Wire container for numeric matrix payloads (validated on decode).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TradingMatrixWire {
    pub rows: usize,
    pub cols: usize,
    pub row_major_flat: Vec<f64>,
}

impl TradingMatrixWire {
    pub fn validated_array(self) -> Result<Array2<f64>, DeserializeTradingStageWireError> {
        let TradingMatrixWire {
            rows,
            cols,
            row_major_flat,
        } = self;
        if rows == 0 || cols == 0 {
            return Err(DeserializeTradingStageWireError::EmptyMatrixDims);
        }
        let cells = rows
            .checked_mul(cols)
            .ok_or(DeserializeTradingStageWireError::MatrixShapeOverflow)?;
        if row_major_flat.len() != cells {
            return Err(
                DeserializeTradingStageWireError::MatrixShapePayloadMismatch {
                    expected: cells,
                    got: row_major_flat.len(),
                },
            );
        }
        Array2::from_shape_vec((rows, cols), row_major_flat).map_err(|_| {
            DeserializeTradingStageWireError::MatrixConstructionFailed
        })
    }

    pub fn try_from_array(array: Array2<f64>) -> Result<Self, SerializeTradingStageError> {
        let shape = array.raw_dim();
        let rows = shape[0];
        let cols = shape[1];
        if rows == 0 || cols == 0 {
            return Err(SerializeTradingStageError::EmptyMatrix);
        }
        let cells = rows
            .checked_mul(cols)
            .ok_or(SerializeTradingStageError::ShapeOverflow)?;
        let mut row_major_flat = Vec::with_capacity(cells);
        for r in 0..rows {
            for c in 0..cols {
                row_major_flat.push(array[[r, c]]);
            }
        }
        Ok(TradingMatrixWire {
            rows,
            cols,
            row_major_flat,
        })
    }
}

/// Strict wire union for machine roundtrips (unknown fields rejected on nested structs).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TradingStageWire {
    pub matrix: TradingMatrixWire,
    pub base: PathPrimitiveWire,
    pub signals: PathPrimitiveWire,
    pub overrides: PathPrimitiveWire,
}

/// Parse / emit errors for strict JSON helpers (wire-level).
#[derive(Debug)]
pub enum TradingStageSerdeError {
    Json(serde_json::Error),
    Wire(DeserializeTradingStageWireError),
    EncodeShape(SerializeTradingStageError),
}

impl fmt::Display for TradingStageSerdeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TradingStageSerdeError::Json(e) => write!(f, "strict JSON parse failed: {e}"),
            TradingStageSerdeError::Wire(e) => write!(f, "wire validation failed: {e}"),
            TradingStageSerdeError::EncodeShape(e) => write!(f, "strict encode failed: {e}"),
        }
    }
}

impl std::error::Error for TradingStageSerdeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            TradingStageSerdeError::Json(e) => Some(e),
            TradingStageSerdeError::Wire(e) => Some(e),
            TradingStageSerdeError::EncodeShape(e) => Some(e),
        }
    }
}

impl From<serde_json::Error> for TradingStageSerdeError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}

impl From<DeserializeTradingStageWireError> for TradingStageSerdeError {
    fn from(value: DeserializeTradingStageWireError) -> Self {
        Self::Wire(value)
    }
}

impl From<SerializeTradingStageError> for TradingStageSerdeError {
    fn from(value: SerializeTradingStageError) -> Self {
        Self::EncodeShape(value)
    }
}

/// Invalid path passed to tier mutators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TierPathError {
    InvalidPath,
}

impl fmt::Display for TierPathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TierPathError::InvalidPath => write!(f, "path must be non-empty dot segments without '..'"),
        }
    }
}

impl std::error::Error for TierPathError {}

/// Validation failure when turning [`TradingStageWire`] into a live [`TradingStage`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeserializeTradingStageWireError {
    EmptyMatrixDims,
    MatrixShapeOverflow,
    MatrixShapePayloadMismatch { expected: usize, got: usize },
    MatrixConstructionFailed,
}

impl fmt::Display for DeserializeTradingStageWireError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DeserializeTradingStageWireError::EmptyMatrixDims => {
                write!(f, "matrix rows/cols must be positive")
            }
            DeserializeTradingStageWireError::MatrixShapeOverflow => {
                write!(f, "matrix row * col overflowed usize")
            }
            DeserializeTradingStageWireError::MatrixShapePayloadMismatch {
                expected,
                got,
            } => write!(
                f,
                "matrix row_major_flat length {got} does not match rows*cols {expected}"
            ),
            DeserializeTradingStageWireError::MatrixConstructionFailed => {
                write!(f, "ndarray could not assume row-major shape")
            }
        }
    }
}

impl std::error::Error for DeserializeTradingStageWireError {}

/// Serialization failure when projecting a live matrix into [`TradingMatrixWire`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SerializeTradingStageError {
    EmptyMatrix,
    ShapeOverflow,
}

impl fmt::Display for SerializeTradingStageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SerializeTradingStageError::EmptyMatrix => write!(f, "matrix dims must be positive"),
            SerializeTradingStageError::ShapeOverflow => write!(f, "matrix row * column overflow"),
        }
    }
}

impl std::error::Error for SerializeTradingStageError {}

/// Ordered list of dot-paths marked dirty (no dedup by default; preserves emission order).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DirtyPathList {
    paths: Vec<String>,
}

impl DirtyPathList {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn mark(&mut self, path: impl Into<String>) {
        self.paths.push(path.into());
    }

    pub fn is_empty(&self) -> bool {
        self.paths.is_empty()
    }

    pub fn len(&self) -> usize {
        self.paths.len()
    }

    pub fn as_slice(&self) -> &[String] {
        &self.paths
    }

    pub fn clear(&mut self) {
        self.paths.clear();
    }

    /// Remove and return all recorded paths.
    pub fn drain(&mut self) -> Vec<String> {
        std::mem::take(&mut self.paths)
    }
}

/// Row/column pairs that need downstream refresh after matrix edits.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DirtyMatrixCellList {
    cells: Vec<(usize, usize)>,
}

impl DirtyMatrixCellList {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn mark(&mut self, row: usize, col: usize) {
        self.cells.push((row, col));
    }

    pub fn mark_all(&mut self, rows: usize, cols: usize) {
        self.cells.clear();
        for r in 0..rows {
            for c in 0..cols {
                self.cells.push((r, c));
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    pub fn as_slice(&self) -> &[(usize, usize)] {
        &self.cells
    }

    pub fn clear(&mut self) {
        self.cells.clear();
    }

    pub fn drain(&mut self) -> Vec<(usize, usize)> {
        std::mem::take(&mut self.cells)
    }
}

/// One tier bucket: path primitives + emitted dirty-path queue.
#[derive(Clone, Debug, Default)]
pub struct TierPrimitiveBucket {
    map: BTreeMap<String, Primitive>,
    dirty_paths: DirtyPathList,
}

impl TierPrimitiveBucket {
    pub fn get(&self, path: &str) -> Option<&Primitive> {
        self.map.get(path)
    }

    pub fn paths(&self) -> impl Iterator<Item = (&String, &Primitive)> {
        self.map.iter()
    }

    pub fn insert_with_mark(
        &mut self,
        path: impl Into<String>,
        primitive: Primitive,
    ) -> Result<Option<Primitive>, TierPathError> {
        let path = path.into();
        validate_path_segments(&path)?;
        let old = self.map.insert(path.clone(), primitive);
        self.dirty_paths.mark(path);
        Ok(old)
    }

    pub fn remove_with_mark(&mut self, path: &str) -> Result<Option<Primitive>, TierPathError> {
        validate_path_segments(path)?;
        let old = self.map.remove(path);
        if old.is_some() {
            self.dirty_paths.mark(path.to_string());
        }
        Ok(old)
    }

    pub fn primitive_dirty_paths_mut(&mut self) -> &mut DirtyPathList {
        &mut self.dirty_paths
    }

    pub fn into_wire(self) -> PathPrimitiveWire {
        PathPrimitiveWire {
            entries: self.map,
        }
    }

    pub fn hydrate_from_wire(wire: PathPrimitiveWire) -> Self {
        TierPrimitiveBucket {
            map: wire.entries,
            dirty_paths: DirtyPathList::default(),
        }
    }
}

/// Tracks which tier pipelines have drained dirty metadata this tick.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LayerPipelineTracker {
    /// Number of merges / logical pipeline passes registered (analytics hook).
    pub pipeline_generation: u64,
    /// Last drained generation snapshot per tier (`None` ⇒ never synced).
    pub last_drained_primitive_epoch: TierEpochMap,
    pub last_drained_matrix_epoch: Option<u64>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TierEpochMap {
    pub base_epoch: Option<u64>,
    pub signals_epoch: Option<u64>,
    pub overrides_epoch: Option<u64>,
}

/// Primary stage matrix + tiered primitives + bookkeeping.
#[derive(Clone, Debug)]
pub struct TradingStage {
    matrix: Array2<f64>,
    base: TierPrimitiveBucket,
    signals: TierPrimitiveBucket,
    overrides: TierPrimitiveBucket,
    matrix_dirty_cells: DirtyMatrixCellList,
    tracker: LayerPipelineTracker,
    matrix_mutation_generation: u64,
    tier_mutation_generation: TierEpochCounters,
}

#[derive(Clone, Debug, Copy, Default)]
struct TierEpochCounters {
    base: u64,
    signals: u64,
    overrides: u64,
}

impl TierEpochCounters {
    fn bump(&mut self, layer: DataStackLayer) -> u64 {
        match layer {
            DataStackLayer::Base => {
                self.base = self.base.saturating_add(1);
                self.base
            }
            DataStackLayer::Signals => {
                self.signals = self.signals.saturating_add(1);
                self.signals
            }
            DataStackLayer::Overrides => {
                self.overrides = self.overrides.saturating_add(1);
                self.overrides
            }
        }
    }

    fn get(&self, layer: DataStackLayer) -> u64 {
        match layer {
            DataStackLayer::Base => self.base,
            DataStackLayer::Signals => self.signals,
            DataStackLayer::Overrides => self.overrides,
        }
    }
}

impl TradingStage {
    pub fn new(matrix: Array2<f64>) -> Self {
        Self {
            matrix,
            base: TierPrimitiveBucket::default(),
            signals: TierPrimitiveBucket::default(),
            overrides: TierPrimitiveBucket::default(),
            matrix_dirty_cells: DirtyMatrixCellList::default(),
            tracker: LayerPipelineTracker::default(),
            matrix_mutation_generation: 0,
            tier_mutation_generation: TierEpochCounters::default(),
        }
    }

    pub fn zeros(rows: usize, cols: usize) -> Result<Self, DeserializeTradingStageWireError> {
        if rows == 0 || cols == 0 {
            return Err(DeserializeTradingStageWireError::EmptyMatrixDims);
        }
        Ok(Self::new(Array2::zeros((rows, cols))))
    }

    pub fn matrix(&self) -> &Array2<f64> {
        &self.matrix
    }

    pub fn matrix_mut(&mut self) -> &mut Array2<f64> {
        let shape = self.matrix.raw_dim();
        self.bump_matrix_epoch();
        self.matrix_dirty_cells.mark_all(shape[0], shape[1]);
        &mut self.matrix
    }

    /// Edit a cell and record deterministic dirty bookkeeping.
    pub fn set_matrix_cell(&mut self, row: usize, col: usize, value: f64) {
        if row < self.matrix.nrows() && col < self.matrix.ncols() {
            self.matrix[(row, col)] = value;
            self.matrix_dirty_cells.mark(row, col);
            self.bump_matrix_epoch();
        }
    }

    pub fn matrix_dirty_cells(&self) -> &DirtyMatrixCellList {
        &self.matrix_dirty_cells
    }

    pub fn matrix_dirty_cells_mut(&mut self) -> &mut DirtyMatrixCellList {
        &mut self.matrix_dirty_cells
    }

    pub fn tracker(&self) -> &LayerPipelineTracker {
        &self.tracker
    }

    pub fn tracker_mut(&mut self) -> &mut LayerPipelineTracker {
        &mut self.tracker
    }

    pub fn bucket(&self, layer: DataStackLayer) -> &TierPrimitiveBucket {
        match layer {
            DataStackLayer::Base => &self.base,
            DataStackLayer::Signals => &self.signals,
            DataStackLayer::Overrides => &self.overrides,
        }
    }

    pub fn bucket_mut(&mut self, layer: DataStackLayer) -> &mut TierPrimitiveBucket {
        match layer {
            DataStackLayer::Base => &mut self.base,
            DataStackLayer::Signals => &mut self.signals,
            DataStackLayer::Overrides => &mut self.overrides,
        }
    }

    /// Insert or overwrite a tier primitive using validated dot-path semantics and bump pipeline epoch data.
    pub fn tier_set_primitive(
        &mut self,
        layer: DataStackLayer,
        path: impl Into<String>,
        primitive: Primitive,
    ) -> Result<Option<Primitive>, TierPathError> {
        let old = match layer {
            DataStackLayer::Base => self.base.insert_with_mark(path, primitive),
            DataStackLayer::Signals => self.signals.insert_with_mark(path, primitive),
            DataStackLayer::Overrides => self.overrides.insert_with_mark(path, primitive),
        }?;
        self.tier_mutation_generation.bump(layer);
        Ok(old)
    }

    /// Drop a keyed primitive if present (epoch bumped only when a value was removed).
    pub fn tier_remove_primitive(
        &mut self,
        layer: DataStackLayer,
        path: &str,
    ) -> Result<Option<Primitive>, TierPathError> {
        let removed = match layer {
            DataStackLayer::Base => self.base.remove_with_mark(path)?,
            DataStackLayer::Signals => self.signals.remove_with_mark(path)?,
            DataStackLayer::Overrides => self.overrides.remove_with_mark(path)?,
        };
        if removed.is_some() {
            self.tier_mutation_generation.bump(layer);
        }
        Ok(removed)
    }

    pub fn tier_epoch_snapshot(&self) -> TierEpochMap {
        TierEpochMap {
            base_epoch: Some(self.tier_mutation_generation.get(DataStackLayer::Base)),
            signals_epoch: Some(self.tier_mutation_generation.get(DataStackLayer::Signals)),
            overrides_epoch: Some(self.tier_mutation_generation.get(DataStackLayer::Overrides)),
        }
    }

    /// Overlay merge read: `Overrides` › `Signals` › `Base`.
    pub fn resolve_primitive(&self, path: &str) -> Option<&Primitive> {
        self.overrides
            .get(path)
            .or_else(|| self.signals.get(path))
            .or_else(|| self.base.get(path))
    }

    fn bump_matrix_epoch(&mut self) {
        self.matrix_mutation_generation = self.matrix_mutation_generation.saturating_add(1);
        self.tracker.pipeline_generation =
            self.tracker.pipeline_generation.saturating_add(1);
    }

    /// Drain queued primitive dirty-path lists for every tier (`Base`, `Signals`, `Overrides`).
    pub fn drain_all_primitive_dirty_paths(&mut self) -> [Vec<String>; 3] {
        self.tracker.last_drained_primitive_epoch = TierEpochMap {
            base_epoch: Some(self.tier_mutation_generation.get(DataStackLayer::Base)),
            signals_epoch: Some(self.tier_mutation_generation.get(DataStackLayer::Signals)),
            overrides_epoch: Some(self.tier_mutation_generation.get(DataStackLayer::Overrides)),
        };
        [
            self.base.primitive_dirty_paths_mut().drain(),
            self.signals.primitive_dirty_paths_mut().drain(),
            self.overrides.primitive_dirty_paths_mut().drain(),
        ]
    }

    pub fn drain_matrix_dirty_cells(&mut self) -> Vec<(usize, usize)> {
        self.tracker.last_drained_matrix_epoch = Some(self.matrix_mutation_generation);
        self.matrix_dirty_cells.drain()
    }
}

/// Strict deserialization from JSON UTF-8.
pub fn deserialize_trading_stage_json_strict(slice: &[u8]) -> Result<TradingStage, TradingStageSerdeError> {
    let wire: TradingStageWire = serde_json::from_slice(slice)?;
    Ok(hydrate_trading_stage_from_wire_strict(wire)?)
}

pub fn hydrate_trading_stage_from_wire_strict(
    wire: TradingStageWire,
) -> Result<TradingStage, DeserializeTradingStageWireError> {
    let matrix = wire.matrix.validated_array()?;
    Ok(TradingStage {
        matrix,
        base: TierPrimitiveBucket::hydrate_from_wire(wire.base),
        signals: TierPrimitiveBucket::hydrate_from_wire(wire.signals),
        overrides: TierPrimitiveBucket::hydrate_from_wire(wire.overrides),
        matrix_dirty_cells: DirtyMatrixCellList::default(),
        tracker: LayerPipelineTracker::default(),
        matrix_mutation_generation: 0,
        tier_mutation_generation: TierEpochCounters::default(),
    })
}

pub fn dehydrate_stage_to_wire(stage: TradingStage) -> Result<TradingStageWire, SerializeTradingStageError> {
    let matrix = TradingMatrixWire::try_from_array(stage.matrix)?;
    Ok(TradingStageWire {
        matrix,
        base: PathPrimitiveWire {
            entries: stage.base.map.into_iter().collect(),
        },
        signals: PathPrimitiveWire {
            entries: stage.signals.map.into_iter().collect(),
        },
        overrides: PathPrimitiveWire {
            entries: stage.overrides.map.into_iter().collect(),
        },
    })
}

/// Strict JSON emission (deterministic map ordering via serde + `serde_json`).
pub fn serialize_trading_stage_json_strict(stage: TradingStage) -> Result<Vec<u8>, TradingStageSerdeError> {
    let wire = dehydrate_stage_to_wire(stage)?;
    Ok(serde_json::to_vec(&wire)?)
}

pub fn serialize_stage_wire_json_pretty(wire: &TradingStageWire) -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec_pretty(wire)
}

pub fn deserialize_stage_wire_json_strict(slice: &[u8]) -> Result<TradingStageWire, serde_json::Error> {
    serde_json::from_slice(slice)
}

// -----------------------------------------------------------------------------
// Owned wire surface for `TradingStage`: manual Serde bridging with validation.
// -----------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TradingStageSerdeProxy {
    matrix: TradingMatrixWire,
    base: PathPrimitiveWire,
    signals: PathPrimitiveWire,
    overrides: PathPrimitiveWire,
}

impl Serialize for TradingStage {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let matrix = TradingMatrixWire::try_from_array(self.matrix.clone())
            .map_err(serde::ser::Error::custom)?;
        let proxy = TradingStageSerdeProxy {
            matrix,
            base: PathPrimitiveWire {
                entries: self.base.map.clone(),
            },
            signals: PathPrimitiveWire {
                entries: self.signals.map.clone(),
            },
            overrides: PathPrimitiveWire {
                entries: self.overrides.map.clone(),
            },
        };
        proxy.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for TradingStage {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let proxy = TradingStageSerdeProxy::deserialize(deserializer)?;
        hydrate_trading_stage_from_wire_strict(TradingStageWire {
            matrix: proxy.matrix,
            base: proxy.base,
            signals: proxy.signals,
            overrides: proxy.overrides,
        })
        .map_err(serde::de::Error::custom)
    }
}
