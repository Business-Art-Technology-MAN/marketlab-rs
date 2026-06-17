//! Decentralized workspace context: thread-safe USD stage handle, UI state, and MVU mutations.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io;
use std::path::Path as FsPath;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use gpui::*;
use openusd::sdf::schema::FieldKey;
use openusd::sdf::{Path, PathListOp, Value};
use openusd::Stage;

use pulsar_marketlab_core::{
    create_and_insert_sublayer, deserialize_portfolio_weights, import_external_portfolio_layer,
    parse_ordered_sublayer_filenames, remove_sublayer, reorder_sublayer, ComposedAssetMeta,
    ComputedAttributeStream,
    ComputedTokenStream, HistoricalTimelineMap, SESSION_LAYER_FILENAME, WORKSTATION_LAYER_STACK,
};

use super::layer_control::{
    build_layer_descriptors, LayerDescriptor, LayerDisplayState, LayerStackControlState,
};
use super::logical_topology::{compile_logical_strategy_tree, LogicalTreeNode};
use super::stage_ledger::StageLedgerEntry;

/// GPUI model update context (GPUI 0.2 [`Context`]).
pub type ModelContext<'a, T> = Context<'a, T>;

/// Workstation panel identifiers for layout focus tracking.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PanelType {
    StageComposer,
    NodeCanvas,
    ParamInspector,
    OtlEditor,
    RenderViewport,
}

/// 2D canvas coordinate keyed by node id.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point2D {
    pub x: f32,
    pub y: f32,
}

/// Passive USD overlay key: property path + SDF field name.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AttributeFieldKey {
    edit_layer: Option<String>,
    property_path: String,
    field: String,
}

/// Passive USD overlay key: prim path + relationship name.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RelationshipKey {
    prim_path: String,
    relationship: String,
}

/// Tracks down-chain signal/portfolio graph nodes needing recomputation.
#[derive(Clone, Debug, Default)]
pub struct ExecutionGraphCache {
    dirty_nodes: HashSet<String>,
}

impl ExecutionGraphCache {
    pub fn dirty_graph_node(&mut self, prim_path: &str) {
        self.dirty_nodes.insert(prim_path.to_string());
    }

    pub fn dirty_nodes(&self) -> &HashSet<String> {
        &self.dirty_nodes
    }

    pub fn take_dirty_nodes(&mut self) -> HashSet<String> {
        std::mem::take(&mut self.dirty_nodes)
    }

    pub fn clear_dirty_nodes(&mut self) {
        self.dirty_nodes.clear();
    }
}

/// Short-lived handle to a prim on the passive USD stage cache.
#[derive(Clone, Debug)]
pub struct ManagedUsdPrim {
    stage: ManagedUsdStage,
    prim_path: String,
}

impl ManagedUsdPrim {
    pub fn path(&self) -> &str {
        &self.prim_path
    }

    /// Write an attribute default opinion into the passive overlay.
    pub fn set_attribute(&self, attr_name: &str, new_value: Value) {
        let property_path = format!("{}.{attr_name}", self.prim_path);
        self.stage.set_field(&property_path, "default", new_value);
    }

    /// Resolve a named relationship on this prim, if one exists on disk or in overlays.
    pub fn get_relationship(&self, name: &str) -> Option<ManagedUsdRelationship> {
        if self.stage.has_relationship(&self.prim_path, name) {
            Some(ManagedUsdRelationship {
                stage: self.stage.clone(),
                prim_path: self.prim_path.clone(),
                name: name.to_string(),
            })
        } else {
            None
        }
    }
}

/// Mutable view of a USD relationship's target path list.
#[derive(Clone, Debug)]
pub struct ManagedUsdRelationship {
    stage: ManagedUsdStage,
    prim_path: String,
    name: String,
}

impl ManagedUsdRelationship {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn get_targets(&self) -> Vec<Path> {
        self.stage
            .relationship_targets(&self.prim_path, &self.name)
            .into_iter()
            .filter_map(|target| Path::new(&target).ok())
            .collect()
    }

    pub fn set_targets(&self, targets: Vec<Path>) {
        let paths = targets.into_iter().map(|path| path.to_string()).collect();
        self.stage
            .set_relationship_targets(&self.prim_path, &self.name, paths);
    }
}

/// Send + Sync handle to a composed OpenUSD root layer.
///
/// Native [`Stage`] uses interior mutability and is `!Send` / `!Sync` in 0.3.0.
/// Structural reads reopen the root layer per call; runtime edits land in passive
/// Passive USD overlays that must survive a full stage recompose from canvas USDA.
#[derive(Clone, Debug, Default)]
pub struct RuntimeOverlaySnapshot {
    pub active_overrides: HashMap<String, bool>,
    pub attribute_overrides: HashMap<AttributeFieldKey, Value>,
    pub relationship_overrides: HashMap<RelationshipKey, Vec<String>>,
    pub edit_target_layer: Option<String>,
}

/// overlay maps that sit above the on-disk layer stack.
#[derive(Clone, Debug)]
pub struct ManagedUsdStage {
    root_layer_path: Arc<String>,
    pub(crate) workstation_layer_stack: bool,
    edit_target_layer: Arc<Mutex<Option<String>>>,
    active_overrides: Arc<Mutex<HashMap<String, bool>>>,
    attribute_overrides: Arc<Mutex<HashMap<AttributeFieldKey, Value>>>,
    relationship_overrides: Arc<Mutex<HashMap<RelationshipKey, Vec<String>>>>,
}

fn is_session_edit_layer(layer: Option<&str>) -> bool {
    match layer {
        None => true,
        Some(id) => {
            id == SESSION_LAYER_FILENAME
                || id.ends_with(&format!("/{SESSION_LAYER_FILENAME}"))
                || std::path::Path::new(id)
                    .file_name()
                    .and_then(|name| name.to_str())
                    == Some(SESSION_LAYER_FILENAME)
        }
    }
}

fn path_list_op_targets(list_op: PathListOp) -> Vec<String> {
    list_op
        .iter()
        .map(|path| path.to_string())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect()
}

impl ManagedUsdStage {
    /// Open and validate a composed stage from a root `.usda` / `.usd` path.
    pub fn open(root_layer_path: impl AsRef<FsPath>) -> io::Result<Self> {
        pulsar_marketlab_core::ensure_schema_sidecar_for_document(root_layer_path.as_ref())?;
        pulsar_marketlab_core::ensure_metadata_library_sidecar_for_document(root_layer_path.as_ref())?;
        let path = root_layer_path.as_ref().to_string_lossy().into_owned();
        Stage::open(&path).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        Ok(Self {
            root_layer_path: Arc::new(path),
            workstation_layer_stack: false,
            edit_target_layer: Arc::new(Mutex::new(None)),
            active_overrides: Arc::new(Mutex::new(HashMap::new())),
            attribute_overrides: Arc::new(Mutex::new(HashMap::new())),
            relationship_overrides: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    /// Write inline USDA text to a temp file, then open it as a composed stage.
    pub fn open_from_usda_text(content: &str) -> io::Result<Self> {
        let path = write_inline_usda(content)?;
        let mut stage = Self::open(path)?;
        stage.workstation_layer_stack = true;
        stage.set_edit_target_layer(Some(SESSION_LAYER_FILENAME.to_string()));
        Ok(stage)
    }

    pub fn root_layer_path(&self) -> &str {
        &self.root_layer_path
    }

    pub fn layer_identifiers(&self) -> Vec<String> {
        if self.workstation_layer_stack {
            if let Ok(text) = fs::read_to_string(self.root_layer_path.as_str()) {
                let parsed = parse_ordered_sublayer_filenames(&text);
                if !parsed.is_empty() {
                    return parsed;
                }
            }
            return WORKSTATION_LAYER_STACK
                .iter()
                .map(|layer| layer.to_string())
                .collect();
        }
        self.with_stage(|stage| Ok(stage.layer_identifiers()))
            .unwrap_or_else(|_| vec![self.root_layer_path.to_string()])
    }

    pub fn create_and_insert_sublayer(&self, filename: &str) -> Result<(), String> {
        create_and_insert_sublayer(FsPath::new(self.root_layer_path.as_str()), filename)
    }

    pub fn reorder_sublayer(&self, from_index: usize, to_index: usize) -> Result<(), String> {
        reorder_sublayer(
            FsPath::new(self.root_layer_path.as_str()),
            from_index,
            to_index,
        )
    }

    pub fn remove_sublayer(&self, filename: &str) -> Result<(), String> {
        remove_sublayer(FsPath::new(self.root_layer_path.as_str()), filename)
    }

    /// Non-destructively copy a portfolio `.usda` into this project and prepend it to the stack.
    pub fn import_portfolio_layer(&self, source_usda_path: &FsPath) -> Result<String, String> {
        import_external_portfolio_layer(
            FsPath::new(self.root_layer_path.as_str()),
            source_usda_path,
        )
    }

    /// Re-open the composed stage from disk after sublayer mutations.
    pub fn reload_from_disk(&self) -> io::Result<Self> {
        let mut stage = Self::open(self.root_layer_path.as_str())?;
        stage.workstation_layer_stack = self.workstation_layer_stack;
        stage.set_edit_target_layer(self.edit_target_layer());
        Ok(stage)
    }

    pub fn edit_target_layer(&self) -> Option<String> {
        self.edit_target_layer
            .lock()
            .ok()
            .and_then(|layer| layer.clone())
    }

    pub fn set_edit_target_layer(&self, layer: Option<String>) {
        if let Ok(mut target) = self.edit_target_layer.lock() {
            *target = layer;
        }
    }

    /// Count of passive attribute field opinions held in memory overlays.
    pub fn overlay_field_count(&self) -> usize {
        self.attribute_overrides
            .lock()
            .ok()
            .map(|overrides| overrides.len())
            .unwrap_or(0)
    }

    /// Approximate in-memory overlay footprint for diagnostics (KiB).
    pub fn has_runtime_overlays(&self) -> bool {
        let active = self
            .active_overrides
            .lock()
            .ok()
            .is_some_and(|map| !map.is_empty());
        let attributes = self.overlay_field_count() > 0;
        let relationships = self
            .relationship_overrides
            .lock()
            .ok()
            .is_some_and(|map| !map.is_empty());
        active || attributes || relationships
    }

    pub fn overlay_memory_kib(&self) -> u64 {
        let fields = self.overlay_field_count() as u64;
        let relationships = self
            .relationship_overrides
            .lock()
            .ok()
            .map(|overrides| overrides.len() as u64)
            .unwrap_or(0);
        ((fields * 96) + (relationships * 128)).div_ceil(1024)
    }

    /// Run a callback against a freshly opened native [`Stage`].
    pub fn with_stage<R>(
        &self,
        f: impl FnOnce(&Stage) -> Result<R, io::Error>,
    ) -> Result<R, io::Error> {
        let stage = Stage::open(self.root_layer_path.as_str())
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        f(&stage)
    }

    /// LIVRPS-composed `active` metadata for a prim path (defaults to `true`).
    pub fn prim_active(&self, prim_path: &str) -> bool {
        if let Some(active) = self
            .active_overrides
            .lock()
            .ok()
            .and_then(|overrides| overrides.get(prim_path).copied())
        {
            return active;
        }
        self.read_prim_active_from_stage(prim_path)
    }

    /// Mutate composed `FieldKey::Active` for a prim path in the passive overlay.
    pub fn set_prim_active(&self, prim_path: &str, active: bool) {
        if let Ok(mut overrides) = self.active_overrides.lock() {
            overrides.insert(prim_path.to_string(), active);
        }
    }

    pub fn snapshot_runtime_overlays(&self) -> RuntimeOverlaySnapshot {
        RuntimeOverlaySnapshot {
            active_overrides: self
                .active_overrides
                .lock()
                .ok()
                .map(|map| map.clone())
                .unwrap_or_default(),
            attribute_overrides: self
                .attribute_overrides
                .lock()
                .ok()
                .map(|map| map.clone())
                .unwrap_or_default(),
            relationship_overrides: self
                .relationship_overrides
                .lock()
                .ok()
                .map(|map| map.clone())
                .unwrap_or_default(),
            edit_target_layer: self.edit_target_layer(),
        }
    }

    pub fn restore_runtime_overlays(&self, snapshot: RuntimeOverlaySnapshot) {
        if let Ok(mut overrides) = self.active_overrides.lock() {
            *overrides = snapshot.active_overrides;
        }
        if let Ok(mut overrides) = self.attribute_overrides.lock() {
            *overrides = snapshot.attribute_overrides;
        }
        if let Ok(mut overrides) = self.relationship_overrides.lock() {
            *overrides = snapshot.relationship_overrides;
        }
        self.set_edit_target_layer(snapshot.edit_target_layer);
    }

    /// Returns a prim handle when the path exists in the composed stage.
    pub fn get_prim_at_path(&self, prim_path: &str) -> Option<ManagedUsdPrim> {
        if !self.prim_exists(prim_path) {
            return None;
        }
        Some(ManagedUsdPrim {
            stage: self.clone(),
            prim_path: prim_path.to_string(),
        })
    }

    pub fn prim_exists(&self, prim_path: &str) -> bool {
        self.with_stage(|stage| {
            Ok(stage
                .has_spec(prim_path)
                .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?)
        })
        .ok()
        .unwrap_or(false)
    }

    pub fn prim_type_name(&self, prim_path: &str) -> Option<String> {
        self.with_stage(|stage| {
            Ok(stage
                .field::<String>(prim_path, FieldKey::TypeName)
                .ok()
                .flatten()
                .map(|token| token.trim_matches('"').to_string())
                .filter(|name| !name.is_empty()))
        })
        .ok()
        .flatten()
    }

    /// Read a composed string attribute default (e.g. `inputs:script_src`).
    pub fn field_string(&self, prim_path: &str, attribute: &str) -> Option<String> {
        let property_path = format!("{prim_path}.{attribute}");
        if let Some(Value::String(text)) = self.field(&property_path, "default") {
            return Some(text);
        }
        self.with_stage(|stage| {
            Ok(stage
                .field::<String>(property_path.as_str(), FieldKey::Default)
                .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?)
        })
        .ok()
        .flatten()
    }

    pub fn has_relationship(&self, prim_path: &str, relationship: &str) -> bool {
        let key = RelationshipKey {
            prim_path: prim_path.to_string(),
            relationship: relationship.to_string(),
        };
        if self
            .relationship_overrides
            .lock()
            .ok()
            .is_some_and(|overrides| overrides.contains_key(&key))
        {
            return true;
        }
        self.with_stage(|stage| {
            let property_path = format!("{prim_path}.{relationship}");
            Ok(stage
                .field::<PathListOp>(property_path.as_str(), FieldKey::TargetPaths)
                .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?
                .is_some())
        })
        .unwrap_or(false)
    }

    pub fn relationship_targets(&self, prim_path: &str, relationship: &str) -> Vec<String> {
        let key = RelationshipKey {
            prim_path: prim_path.to_string(),
            relationship: relationship.to_string(),
        };
        if let Ok(overrides) = self.relationship_overrides.lock() {
            if let Some(targets) = overrides.get(&key) {
                return targets.clone();
            }
        }
        let property_path = format!("{prim_path}.{relationship}");
        self.with_stage(|stage| {
            Ok(stage
                .field::<PathListOp>(property_path.as_str(), FieldKey::TargetPaths)
                .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?
                .map(path_list_op_targets)
                .unwrap_or_default())
        })
        .unwrap_or_default()
    }

    pub fn set_relationship_targets(
        &self,
        prim_path: &str,
        relationship: &str,
        targets: Vec<String>,
    ) {
        if self.workstation_layer_stack
            && !is_session_edit_layer(self.edit_target_layer().as_deref())
        {
            return;
        }
        if let Ok(mut overrides) = self.relationship_overrides.lock() {
            overrides.insert(
                RelationshipKey {
                    prim_path: prim_path.to_string(),
                    relationship: relationship.to_string(),
                },
                targets,
            );
        }
    }

    /// Write a field opinion into the passive USD memory overlay for the active edit target.
    pub fn set_field(&self, property_path: &str, field: &str, val: Value) {
        let edit_layer = self.edit_target_layer();
        if self.workstation_layer_stack && !is_session_edit_layer(edit_layer.as_deref()) {
            return;
        }
        if let Ok(mut overrides) = self.attribute_overrides.lock() {
            overrides.insert(
                AttributeFieldKey {
                    edit_layer,
                    property_path: property_path.to_string(),
                    field: field.to_string(),
                },
                val,
            );
        }
    }

    /// Resolve a composed field, checking edit-target overlays before the on-disk stage.
    pub fn field(&self, property_path: &str, field: &str) -> Option<Value> {
        let edit_layer = self.edit_target_layer();
        if let Ok(overrides) = self.attribute_overrides.lock() {
            let scoped = AttributeFieldKey {
                edit_layer: edit_layer.clone(),
                property_path: property_path.to_string(),
                field: field.to_string(),
            };
            if let Some(val) = overrides.get(&scoped) {
                return Some(val.clone());
            }
            let legacy = AttributeFieldKey {
                edit_layer: None,
                property_path: property_path.to_string(),
                field: field.to_string(),
            };
            if let Some(val) = overrides.get(&legacy) {
                return Some(val.clone());
            }
        }
        self.with_stage(|stage| {
            Ok(stage
                .field::<Value>(property_path, field)
                .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?)
        })
        .ok()
        .flatten()
    }

    fn read_prim_active_from_stage(&self, prim_path: &str) -> bool {
        self.with_stage(|stage| {
            Ok(stage
                .field::<bool>(prim_path, FieldKey::Active)
                .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?
                .unwrap_or(true))
        })
        .unwrap_or(true)
    }
}

/// Shared workstation context model for MVU panes.
pub struct WorkspaceContext {
    usd_stage: ManagedUsdStage,
    selected_path: Option<String>,
    /// Monotonic flag stepped on every unified selection mutation (tree + canvas).
    ui_selection_generation: u64,
    node_positions: HashMap<String, Point2D>,
    active_panels: Vec<PanelType>,
    ledger_entries: Vec<StageLedgerEntry>,
    /// Monotonic flag stepped whenever [`Self::ledger_entries`] changes.
    ledger_generation: u64,
    computed_streams: Vec<ComputedAttributeStream>,
    computed_token_streams: Vec<pulsar_marketlab_core::ComputedTokenStream>,
    asset_registry: HashMap<String, ComposedAssetMeta>,
    engine_cache_generation: u64,
    execution_engine: ExecutionGraphCache,
    /// Visual edit-target layer (OpenUSD edit targets are not yet exposed in openusd 0.3).
    edit_target_layer: Option<String>,
    /// Layer opinion tray: mute / isolate / reorder for the dopesheet header.
    layer_stack: LayerStackControlState,
    /// O(1) frame → calendar date map for chronological timeline rulers.
    historical_timeline_map: HistoricalTimelineMap,
    /// Monotonic counter for auto-generated user sublayer filenames.
    user_layer_serial: u64,
}

impl WorkspaceContext {
    pub fn new(root_layer_path: impl AsRef<FsPath>) -> io::Result<Self> {
        let mut usd_stage = ManagedUsdStage::open(root_layer_path)?;
        let edit_target_layer = Some(SESSION_LAYER_FILENAME.to_string());
        usd_stage.workstation_layer_stack = true;
        usd_stage.set_edit_target_layer(edit_target_layer.clone());
        Ok(Self {
            usd_stage,
            selected_path: None,
            ui_selection_generation: 0,
            node_positions: HashMap::new(),
            active_panels: default_active_panels(),
            ledger_entries: Vec::new(),
            ledger_generation: 0,
            computed_streams: Vec::new(),
            computed_token_streams: Vec::new(),
            asset_registry: HashMap::new(),
            engine_cache_generation: 0,
            execution_engine: ExecutionGraphCache::default(),
            edit_target_layer,
            layer_stack: LayerStackControlState::new(),
            historical_timeline_map: HistoricalTimelineMap::default(),
            user_layer_serial: 0,
        })
    }

    pub fn from_usda_text(content: &str) -> io::Result<Self> {
        let usd_stage = ManagedUsdStage::open_from_usda_text(content)?;
        let edit_target_layer = Some(SESSION_LAYER_FILENAME.to_string());
        Ok(Self {
            usd_stage,
            selected_path: None,
            ui_selection_generation: 0,
            node_positions: HashMap::new(),
            active_panels: default_active_panels(),
            ledger_entries: Vec::new(),
            ledger_generation: 0,
            computed_streams: Vec::new(),
            computed_token_streams: Vec::new(),
            asset_registry: HashMap::new(),
            engine_cache_generation: 0,
            execution_engine: ExecutionGraphCache::default(),
            edit_target_layer,
            layer_stack: LayerStackControlState::new(),
            historical_timeline_map: HistoricalTimelineMap::default(),
            user_layer_serial: 0,
        })
    }

    /// Reapply runtime overlays captured before a canvas-driven USDA recompose.
    pub fn restore_runtime_overlays_from(&mut self, snapshot: RuntimeOverlaySnapshot) {
        self.usd_stage.restore_runtime_overlays(snapshot.clone());
        self.edit_target_layer = snapshot.edit_target_layer;
    }

    pub fn usd_stage(&self) -> &ManagedUsdStage {
        &self.usd_stage
    }

    pub fn layer_identifiers(&self) -> Vec<String> {
        self.usd_stage.layer_identifiers()
    }

    pub fn edit_target_layer(&self) -> Option<&str> {
        self.edit_target_layer.as_deref()
    }

    pub fn set_edit_target_layer(
        &mut self,
        layer: Option<String>,
        cx: &mut ModelContext<Self>,
    ) {
        if self.edit_target_layer == layer {
            return;
        }
        self.edit_target_layer = layer.clone();
        self.usd_stage.set_edit_target_layer(layer);
        cx.notify();
    }

    pub fn selected_path(&self) -> Option<&str> {
        self.selected_path.as_deref()
    }

    pub fn ui_selection_generation(&self) -> u64 {
        self.ui_selection_generation
    }

    pub fn node_positions(&self) -> &HashMap<String, Point2D> {
        &self.node_positions
    }

    pub fn active_panels(&self) -> &[PanelType] {
        &self.active_panels
    }

    pub fn engine_cache_generation(&self) -> u64 {
        self.engine_cache_generation
    }

    pub fn is_engine_cache_dirty(&self, last_compiled_generation: u64) -> bool {
        self.engine_cache_generation != last_compiled_generation
    }

    pub fn computed_streams(&self) -> &[ComputedAttributeStream] {
        &self.computed_streams
    }

    pub fn replace_computed_streams(&mut self, streams: Vec<ComputedAttributeStream>) {
        self.computed_streams = streams;
        self.execution_engine.clear_dirty_nodes();
    }

    pub fn computed_token_streams(&self) -> &[ComputedTokenStream] {
        &self.computed_token_streams
    }

    pub fn replace_computed_token_streams(&mut self, streams: Vec<ComputedTokenStream>) {
        self.computed_token_streams = streams;
    }

    pub fn asset_registry(&self) -> &HashMap<String, ComposedAssetMeta> {
        &self.asset_registry
    }

    pub fn composed_asset_for_path(&self, prim_path: &str) -> Option<&ComposedAssetMeta> {
        self.asset_registry.get(prim_path)
    }

    pub fn replace_asset_registry(&mut self, registry: HashMap<String, ComposedAssetMeta>) {
        self.asset_registry = registry;
    }

    pub fn execution_engine(&self) -> &ExecutionGraphCache {
        &self.execution_engine
    }

    pub fn execution_engine_mut(&mut self) -> &mut ExecutionGraphCache {
        &mut self.execution_engine
    }

    pub fn ledger_entries(&self) -> &[StageLedgerEntry] {
        &self.ledger_entries
    }

    pub fn ledger_generation(&self) -> u64 {
        self.ledger_generation
    }

    pub fn replace_ledger_entries(&mut self, entries: Vec<StageLedgerEntry>) {
        if self.ledger_entries == entries {
            return;
        }
        self.ledger_entries = entries;
        self.ledger_generation = self.ledger_generation.wrapping_add(1);
    }

    pub fn set_selected_path(&mut self, path: Option<String>, cx: &mut ModelContext<Self>) {
        if self.selected_path == path {
            return;
        }
        self.selected_path = path;
        self.ui_selection_generation = self.ui_selection_generation.wrapping_add(1);
        cx.notify();
    }

    pub fn set_node_position(
        &mut self,
        node_id: impl Into<String>,
        position: Point2D,
        cx: &mut ModelContext<Self>,
    ) {
        self.node_positions.insert(node_id.into(), position);
        cx.notify();
    }

    pub fn set_active_panels(&mut self, panels: Vec<PanelType>, cx: &mut ModelContext<Self>) {
        self.active_panels = panels;
        cx.notify();
    }

    /// Commits a primitive attribute change to the USD stage and notifies the application.
    pub fn modify_attribute(
        &mut self,
        prim_path: &str,
        attr_name: &str,
        new_value: Value,
        cx: &mut ModelContext<Self>,
    ) {
        if attr_name == "inputs:active" {
            if let Value::Bool(active) = new_value {
                self.usd_stage.set_prim_active(prim_path, active);
                let property_path = format!("{prim_path}.{attr_name}");
                self.usd_stage
                    .set_field(&property_path, "default", Value::Bool(active));
            }
        } else if let Some(prim) = self.usd_stage.get_prim_at_path(prim_path) {
            prim.set_attribute(attr_name, new_value);
        } else {
            return;
        }

        self.execution_engine.dirty_graph_node(prim_path);
        self.invalidate_engine_cache(cx);
        cx.notify();
    }

    /// Mutates an open relationship array (used for node wiring actions).
    pub fn connect_primitives(
        &mut self,
        source_path: &str,
        target_prim_path: &str,
        cx: &mut ModelContext<Self>,
    ) {
        use super::node_canvas::{
            compile_relationship_directive, execution_slot_for_target_prim,
            execution_slot_for_target_type,
        };

        let slot = self
            .usd_stage
            .prim_type_name(target_prim_path)
            .as_deref()
            .and_then(execution_slot_for_target_type)
            .or_else(|| execution_slot_for_target_prim(target_prim_path));
        let Some(slot) = slot else {
            return;
        };
        let directive = compile_relationship_directive(target_prim_path, source_path, slot);
        let Some(target_prim) = self.usd_stage.get_prim_at_path(&directive.target_prim_path) else {
            return;
        };
        let Some(relationship) = target_prim.get_relationship(&directive.relationship) else {
            if !self
                .usd_stage
                .has_relationship(&directive.target_prim_path, &directive.relationship)
            {
                return;
            }
            let mut targets = self
                .usd_stage
                .relationship_targets(&directive.target_prim_path, &directive.relationship);
            if targets.iter().any(|target| target == &directive.source_prim_path) {
                return;
            }
            targets.push(directive.source_prim_path.clone());
            self.usd_stage.set_relationship_targets(
                &directive.target_prim_path,
                &directive.relationship,
                targets,
            );
            self.execution_engine
                .dirty_graph_node(&directive.target_prim_path);
            self.invalidate_engine_cache(cx);
            cx.notify();
            return;
        };

        let mut targets = relationship.get_targets();
        let Ok(new_target) = Path::new(&directive.source_prim_path) else {
            return;
        };
        if targets.iter().any(|target| target == &new_target) {
            return;
        }

        targets.push(new_target);
        relationship.set_targets(targets);
        self.execution_engine
            .dirty_graph_node(&directive.target_prim_path);
        self.invalidate_engine_cache(cx);
        cx.notify();
    }

    /// MVU transaction: write a USD attribute default down to the passive overlay,
    /// invalidate engine caches on the background pool, and repaint the window.
    pub fn set_usd_attribute(
        &mut self,
        prim_path: &str,
        attr: &str,
        val: Value,
        cx: &mut ModelContext<Self>,
    ) {
        self.modify_attribute(prim_path, attr, val, cx);
    }

    pub fn set_prim_active(&mut self, prim_path: &str, active: bool, cx: &mut ModelContext<Self>) {
        self.usd_stage.set_prim_active(prim_path, active);
        self.execution_engine.dirty_graph_node(prim_path);
        self.invalidate_engine_cache(cx);
        cx.notify();
    }

    pub fn invalidate_engine_cache(&mut self, _cx: &mut ModelContext<Self>) {
        self.engine_cache_generation = self.engine_cache_generation.wrapping_add(1);
    }

    /// Topology or wiring changed — full graph recompile required.
    pub fn invalidate_engine_topology_cache(&mut self, cx: &mut ModelContext<Self>) {
        self.invalidate_engine_cache(cx);
        cx.notify();
    }

    pub fn get_active_layer_stack_descriptors(&self) -> Vec<LayerDescriptor> {
        build_layer_descriptors(&self.layer_stack)
    }

    pub fn toggle_layer_mute(&mut self, filename: &str, cx: &mut ModelContext<Self>) {
        self.layer_stack.toggle_mute(filename);
        cx.notify();
    }

    pub fn reorder_layer(&mut self, from: usize, to: usize, cx: &mut ModelContext<Self>) {
        self.layer_stack.reorder(from, to);
        cx.notify();
    }

    pub fn historical_timeline_map(&self) -> &HistoricalTimelineMap {
        &self.historical_timeline_map
    }

    pub fn set_historical_timeline_map(&mut self, map: HistoricalTimelineMap) {
        self.historical_timeline_map = map;
    }

    pub fn sync_layer_stack_from_stage(&mut self) {
        let layers = self.layer_identifiers();
        self.layer_stack.sync_ordered_layers(layers);
    }

    pub fn next_user_layer_filename(&mut self) -> String {
        self.user_layer_serial = self.user_layer_serial.saturating_add(1);
        format!("user_layer_{}.usda", self.user_layer_serial)
    }

    pub fn create_workspace_sublayer(
        &mut self,
        filename: &str,
        cx: &mut ModelContext<Self>,
    ) -> Result<(), String> {
        self.usd_stage.create_and_insert_sublayer(filename)?;
        self.sync_layer_stack_from_stage();
        self.set_edit_target_layer(Some(filename.to_string()), cx);
        self.invalidate_engine_cache(cx);
        cx.notify();
        Ok(())
    }

    pub fn reorder_workspace_sublayer(
        &mut self,
        from_index: usize,
        to_index: usize,
        cx: &mut ModelContext<Self>,
    ) -> Result<(), String> {
        self.usd_stage.reorder_sublayer(from_index, to_index)?;
        self.sync_layer_stack_from_stage();
        self.invalidate_engine_cache(cx);
        cx.notify();
        Ok(())
    }

    /// Import a portfolio graph from another project as a non-destructive sublayer.
    pub fn import_portfolio_layer_into_workspace(
        &mut self,
        source_usda_path: &FsPath,
        cx: &mut ModelContext<Self>,
    ) -> Result<String, String> {
        let filename = self.usd_stage.import_portfolio_layer(source_usda_path)?;
        self.usd_stage = self
            .usd_stage
            .reload_from_disk()
            .map_err(|err| format!("Failed to reload stage after import: {err}"))?;
        self.usd_stage.workstation_layer_stack = true;
        self.sync_layer_stack_from_stage();
        self.invalidate_engine_cache(cx);
        cx.notify();
        Ok(filename)
    }

    pub fn is_structured_portfolio_mode(&self) -> bool {
        super::logical_topology::stage_has_portfolio_integrators(&self.usd_stage)
    }

    pub fn is_loose_otl_sandbox_mode(&self) -> bool {
        !self.is_structured_portfolio_mode()
    }

    pub fn remove_sublayer_from_workspace(
        &mut self,
        filename: &str,
        cx: &mut ModelContext<Self>,
    ) -> Result<(), String> {
        if filename == SESSION_LAYER_FILENAME {
            return Err("Cannot delete the session edit layer.".to_string());
        }
        self.usd_stage.remove_sublayer(filename)?;
        self.sync_layer_stack_from_stage();
        if self.edit_target_layer.as_deref() == Some(filename) {
            self.set_edit_target_layer(Some(SESSION_LAYER_FILENAME.to_string()), cx);
        }
        self.invalidate_engine_cache(cx);
        cx.notify();
        Ok(())
    }

    pub fn is_layer_active(&self, filename: &str) -> bool {
        self.layer_stack.state_for(filename) != LayerDisplayState::Muted
    }

    pub fn set_layer_active_state(
        &mut self,
        filename: &str,
        active: bool,
        cx: &mut ModelContext<Self>,
    ) {
        let currently_active = self.is_layer_active(filename);
        if currently_active != active {
            self.layer_stack.toggle_mute(filename);
            cx.notify();
        }
    }

    pub fn get_ordered_sublayers(&self) -> Vec<String> {
        self.layer_identifiers()
    }

    pub fn set_selected_active_target_layer(
        &mut self,
        filename: &str,
        cx: &mut ModelContext<Self>,
    ) {
        self.set_edit_target_layer(Some(filename.to_string()), cx);
    }

    pub fn compile_logical_strategy_tree(&self) -> Vec<LogicalTreeNode> {
        compile_logical_strategy_tree(&self.usd_stage, &self.asset_registry)
    }

    pub fn is_selected(&self, prim_path: &str) -> bool {
        self.selected_path.as_deref() == Some(prim_path)
    }

    pub fn is_node_enabled_in_stage(&self, prim_path: &str) -> bool {
        self.usd_stage.prim_active(prim_path)
    }

    pub fn set_node_enabled_in_stage(
        &mut self,
        prim_path: &str,
        enabled: bool,
        cx: &mut ModelContext<Self>,
    ) {
        self.set_prim_active(prim_path, enabled, cx);
    }

    pub fn get_dominating_layer_for_prim(&self, prim_path: &str) -> Option<String> {
        let descriptors = self.get_active_layer_stack_descriptors();
        let active_layers: Vec<_> = descriptors
            .iter()
            .filter(|layer| layer.state != LayerDisplayState::Muted)
            .collect();
        let layers = if active_layers
            .iter()
            .any(|layer| layer.state == LayerDisplayState::Isolated)
        {
            active_layers
                .into_iter()
                .filter(|layer| layer.state == LayerDisplayState::Isolated)
                .collect()
        } else {
            active_layers
        };

        for layer in &layers {
            if layer.filename == SESSION_LAYER_FILENAME {
                if self
                    .usd_stage
                    .snapshot_runtime_overlays()
                    .attribute_overrides
                    .keys()
                    .any(|key| key.property_path.starts_with(prim_path))
                {
                    return Some(layer.display_name.clone());
                }
            }
            if layer.filename.contains("signals") && self.usd_stage.prim_exists(prim_path) {
                if self
                    .usd_stage
                    .prim_type_name(prim_path)
                    .is_some_and(|type_name| {
                        type_name == "OtlOperator"
                            || type_name == "OtlTaUberSignal"
                            || type_name == "PortfolioIntegrator"
                    })
                {
                    return Some(layer.display_name.clone());
                }
            }
            if layer.filename.contains("sp500") || layer.filename.contains("universe") {
                if self
                    .usd_stage
                    .prim_type_name(prim_path)
                    .is_some_and(|type_name| type_name == "FinancialAsset")
                {
                    return Some(layer.display_name.clone());
                }
            }
        }
        layers.first().map(|layer| layer.display_name.clone())
    }

    pub fn query_frame_metric_string(&self, prim_path: &str, frame_idx: usize) -> String {
        self.query_dopesheet_cell_display(prim_path, frame_idx)
    }

    /// Type-aware dopesheet cell metric: portfolio NAV vs leaf allocation weight %.
    pub fn query_dopesheet_cell_display(&self, prim_path: &str, frame_idx: usize) -> String {
        match self.usd_stage.prim_type_name(prim_path).as_deref() {
            Some("PortfolioIntegrator") => self
                .query_portfolio_nav_at_frame(prim_path, frame_idx)
                .unwrap_or_else(|| "---".to_string()),
            Some("FinancialAsset") => self
                .query_asset_weight_at_frame(prim_path, frame_idx)
                .map(|weight| format!("{weight:.1}%"))
                .unwrap_or_else(|| "0.0%".to_string()),
            _ => self
                .numeric_stream_display(prim_path, frame_idx)
                .unwrap_or_else(|| "—".to_string()),
        }
    }

    pub fn query_portfolio_nav_at_frame(
        &self,
        prim_path: &str,
        frame_idx: usize,
    ) -> Option<String> {
        self.numeric_stream_value_for_attribute(prim_path, "outputs:portfolio_wealth", frame_idx)
            .map(|value| format!("{value:.0}"))
    }

    pub fn query_asset_weight_at_frame(&self, prim_path: &str, frame_idx: usize) -> Option<f64> {
        let leaf = prim_path.rsplit('/').next().filter(|s| !s.is_empty());
        for stream in &self.computed_token_streams {
            if stream.attribute != "outputs:weights" {
                continue;
            }
            let Some(encoded) = token_sample_at_frame(stream, frame_idx) else {
                continue;
            };
            let weights = deserialize_portfolio_weights(encoded);
            if let Some(weight) = weights.get(prim_path) {
                return Some(*weight * 100.0);
            }
            if let Some(leaf) = leaf {
                if let Some((_, weight)) = weights
                    .iter()
                    .find(|(path, _)| path.rsplit('/').next() == Some(leaf))
                {
                    return Some(*weight * 100.0);
                }
            }
        }
        None
    }

    fn numeric_stream_display(&self, prim_path: &str, frame_idx: usize) -> Option<String> {
        for attribute in [
            "outputs:signal",
            "outputs:portfolio_wealth",
            "outputs:weights",
        ] {
            if let Some(value) =
                self.numeric_stream_value_for_attribute(prim_path, attribute, frame_idx)
            {
                return Some(format!("{value:.2}"));
            }
        }
        None
    }

    fn numeric_stream_display_for_attribute(
        &self,
        prim_path: &str,
        attribute: &str,
        frame_idx: usize,
    ) -> Option<String> {
        self.numeric_stream_value_for_attribute(prim_path, attribute, frame_idx)
            .map(|value| format!("{value:.2}"))
    }

    fn numeric_stream_value_for_attribute(
        &self,
        prim_path: &str,
        attribute: &str,
        frame_idx: usize,
    ) -> Option<f64> {
        let stream = self.computed_streams.iter().find(|stream| {
            stream.prim_path == prim_path && stream.attribute == attribute
        })?;
        numeric_sample_at_frame(stream, frame_idx)
    }
}

fn numeric_sample_at_frame(stream: &ComputedAttributeStream, frame_idx: usize) -> Option<f64> {
    stream.values.get(frame_idx).copied()
}

fn token_sample_at_frame(stream: &ComputedTokenStream, frame_idx: usize) -> Option<&str> {
    stream
        .samples
        .iter()
        .find(|(bar, _)| *bar as usize == frame_idx)
        .map(|(_, value)| value.as_str())
        .or_else(|| stream.samples.get(frame_idx).map(|(_, value)| value.as_str()))
}

impl Default for WorkspaceContext {
    fn default() -> Self {
        Self::from_usda_text("#usda 1.0\n").unwrap_or_else(|_| Self {
            usd_stage: ManagedUsdStage {
                root_layer_path: Arc::new(String::new()),
                workstation_layer_stack: true,
                edit_target_layer: Arc::new(Mutex::new(Some(
                    SESSION_LAYER_FILENAME.to_string(),
                ))),
                active_overrides: Arc::new(Mutex::new(HashMap::new())),
                attribute_overrides: Arc::new(Mutex::new(HashMap::new())),
                relationship_overrides: Arc::new(Mutex::new(HashMap::new())),
            },
            selected_path: None,
            ui_selection_generation: 0,
            node_positions: HashMap::new(),
            active_panels: default_active_panels(),
            ledger_entries: Vec::new(),
            ledger_generation: 0,
            computed_streams: Vec::new(),
            computed_token_streams: Vec::new(),
            asset_registry: HashMap::new(),
            engine_cache_generation: 0,
            execution_engine: ExecutionGraphCache::default(),
            edit_target_layer: None,
            layer_stack: LayerStackControlState::new(),
            historical_timeline_map: HistoricalTimelineMap::default(),
            user_layer_serial: 0,
        })
    }
}

/// Repaint pane hosts when the unified USD path selection changes.
pub fn install_ui_selection_observer<H: 'static>(
    workspace: &Entity<WorkspaceContext>,
    cx: &mut Context<H>,
) {
    use std::cell::Cell;
    use std::rc::Rc;

    let last_generation = Rc::new(Cell::new(workspace.read(cx).ui_selection_generation()));
    let tracked = last_generation.clone();
    cx.observe(workspace, move |_host, workspace, cx| {
        let generation = workspace.read(cx).ui_selection_generation();
        if generation != tracked.get() {
            tracked.set(generation);
            cx.notify();
        }
    })
    .detach();
}

fn default_active_panels() -> Vec<PanelType> {
    vec![
        PanelType::StageComposer,
        PanelType::NodeCanvas,
        PanelType::ParamInspector,
        PanelType::OtlEditor,
        PanelType::RenderViewport,
    ]
}

static INLINE_USDA_COUNTER: AtomicU64 = AtomicU64::new(0);

fn write_inline_usda(content: &str) -> io::Result<String> {
    let dir = std::env::temp_dir().join("marketlab_openusd");
    fs::create_dir_all(&dir)?;
    let unique = INLINE_USDA_COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = dir.join(format!(
        "inline_{}_{unique}.usda",
        std::process::id()
    ));
    fs::write(&path, content)?;
    Ok(path.to_string_lossy().into_owned())
}
