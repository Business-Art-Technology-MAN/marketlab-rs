//! Background graph-engine invalidation worker for the workstation layout.

use std::collections::HashMap;

use gpui::*;
use openusd::sdf::schema::FieldKey;
use openusd::sdf::Path;
use pulsar_marketlab_core::{
    ComposedAssetMeta, GraphCompileWire, MarketLabGraphEngine, PathBindingIndex, SharedPriceColumn,
    StageGraphPrim, StageGraphSnapshot, TimelineExecutionResult, USER_LABEL_ATTR,
};

use super::context::{ManagedUsdStage, WorkspaceContext};

const LINEAGE_RELATIONSHIPS: &[&str] = &[
    "inputs:underlying",
    "inputs:sources",
    "inputs:constituents",
    "inputs:target",
];

/// Host hook supplying asset vectors and receiving timeline sweep results.
pub trait GraphEngineInvalidationHost: Sized + 'static {
    fn workspace_context(&self) -> &Entity<WorkspaceContext>;

    fn graph_engine_bootstrapping(&self) -> bool {
        false
    }

    fn graph_engine_asset_vectors(&self) -> HashMap<String, SharedPriceColumn>;

    fn graph_engine_timeline_len(&self) -> usize;

    fn graph_engine_last_compiled_generation(&self) -> u64;

    fn set_graph_engine_last_compiled_generation(&mut self, generation: u64);

    fn graph_engine_recompile_inflight(&self) -> bool;

    fn set_graph_engine_recompile_inflight(&mut self, inflight: bool);

    fn graph_engine_recompile_pending(&self) -> bool;

    fn set_graph_engine_recompile_pending(&mut self, pending: bool);

    fn graph_engine_last_compile_ms(&self) -> u64 {
        0
    }

    fn set_graph_engine_last_compile_ms(&mut self, _ms: u64) {}

    fn graph_engine_asset_data_epoch(&self) -> u64 {
        0
    }

    fn graph_engine_last_swept_asset_epoch(&self) -> u64 {
        0
    }

    fn set_graph_engine_last_swept_asset_epoch(&mut self, _epoch: u64) {}

    fn graph_engine_compile_error(&self) -> Option<String> {
        None
    }

    fn set_graph_engine_compile_error(&mut self, _error: Option<String>) {}

    fn take_cached_graph_engine(
        &mut self,
        _generation: u64,
    ) -> Option<MarketLabGraphEngine> {
        None
    }

    fn store_cached_graph_engine(
        &mut self,
        _generation: u64,
        _engine: MarketLabGraphEngine,
    ) {
    }

    fn on_graph_engine_compile_failed(&mut self, _error: String, _cx: &mut Context<Self>) {}

    fn graph_engine_invalidation_deferred(&self) -> bool {
        false
    }

    type UiSnapshotBuildInput: Clone + Send + 'static;
    type UiSnapshot: Send + Sync + 'static;

    fn graph_ui_snapshot_build_input(&self, cx: &App) -> Self::UiSnapshotBuildInput;

    fn build_graph_ui_snapshot(
        result: &TimelineExecutionResult,
        input: &Self::UiSnapshotBuildInput,
    ) -> Self::UiSnapshot;

    fn apply_graph_engine_timeline_result(
        &mut self,
        result: TimelineExecutionResult,
        ui_snapshot: Option<std::sync::Arc<Self::UiSnapshot>>,
        cx: &mut Context<Self>,
    );

    /// Canvas topology revision used to cache compiled stage snapshots.
    fn graph_engine_pipeline_revision(&self) -> u64 {
        0
    }

    /// Cached stage snapshot for the current pipeline revision, if already built.
    fn graph_engine_cached_stage_snapshot(&self) -> Option<std::sync::Arc<StageGraphSnapshot>> {
        None
    }

    /// Store a freshly built stage snapshot for reuse across asset-only re-sweeps.
    fn graph_engine_store_stage_snapshot(
        &mut self,
        _revision: u64,
        _snapshot: std::sync::Arc<StageGraphSnapshot>,
    ) {
    }

    /// Compose in-memory USDA for canvas topology (legacy fallback; prefer [`graph_engine_direct_stage_snapshot`]).
    fn graph_engine_compose_stage_usda(
        &self,
        _stage: &ManagedUsdStage,
        _cx: &App,
    ) -> Option<String> {
        None
    }

    /// Build a stage snapshot directly from in-memory canvas topology (no USDA round-trip).
    fn graph_engine_direct_stage_snapshot(&self, _cx: &App) -> Option<StageGraphSnapshot> {
        None
    }

    fn graph_engine_stage_snapshot(
        &self,
        stage: &ManagedUsdStage,
        _cx: &App,
    ) -> StageGraphSnapshot {
        build_stage_graph_snapshot(stage)
    }

    fn spawn_graph_engine_recompile(&mut self, cx: &mut Context<Self>) {
        begin_graph_engine_timeline_sweep(self, cx.entity(), cx);
    }
}

/// Dispatch a background graph-engine vectorized timeline execution and bounce results to UI state.
pub fn spawn_graph_engine_timeline_sweep<H: GraphEngineInvalidationHost>(
    view: Entity<H>,
    cx: &mut Context<H>,
) {
    let entity = view.clone();
    cx.defer(move |cx| {
        entity.update(cx, |host, cx| {
            begin_graph_engine_timeline_sweep(host, entity.clone(), cx);
        });
    });
}

/// Same as [`spawn_graph_engine_timeline_sweep`] but assumes the host is already mutably borrowed.
pub fn begin_graph_engine_timeline_sweep<H: GraphEngineInvalidationHost>(
    host: &mut H,
    view: Entity<H>,
    cx: &mut Context<H>,
) {
    let _ = view;
    let context_handle = host.workspace_context().clone();
    let generation = context_handle.read(cx).engine_cache_generation();
    let asset_epoch = host.graph_engine_asset_data_epoch();
    if generation == host.graph_engine_last_compiled_generation()
        && asset_epoch == host.graph_engine_last_swept_asset_epoch()
    {
        return;
    }
    if host.graph_engine_recompile_inflight() {
        host.set_graph_engine_recompile_pending(true);
        return;
    }
    if host.graph_engine_invalidation_deferred() {
        host.set_graph_engine_recompile_pending(true);
        return;
    }

    host.set_graph_engine_recompile_inflight(true);
    let asset_vectors = host.graph_engine_asset_vectors();
    let timeline_len = host.graph_engine_timeline_len();
    let stage = context_handle.read(cx).usd_stage().clone();
    let pipeline_revision = host.graph_engine_pipeline_revision();
    let cached_stage_snapshot = host.graph_engine_cached_stage_snapshot();
    let direct_stage_snapshot = if cached_stage_snapshot.is_none() {
        host.graph_engine_direct_stage_snapshot(cx).map(std::sync::Arc::new)
    } else {
        None
    };
    let stage_usda = if cached_stage_snapshot.is_none() && direct_stage_snapshot.is_none() {
        host.graph_engine_compose_stage_usda(&stage, cx)
    } else {
        None
    };
    let ui_build_input = host.graph_ui_snapshot_build_input(cx);
    let target_generation = generation;
    let target_asset_epoch = asset_epoch;
    let last_compiled_generation = host.graph_engine_last_compiled_generation();
    let reuse_compiled_engine = target_generation == last_compiled_generation;
    let cached_engine = if reuse_compiled_engine {
        host.take_cached_graph_engine(target_generation)
    } else {
        None
    };

    cx.spawn(async move |this, cx| {
        let started = std::time::Instant::now();
        let (timeline_result, compile_error, asset_registry, compiled_engine, ui_snapshot, built_snapshot) = cx
            .background_executor()
            .spawn(async move {
                let (snapshot, built_snapshot) = if let Some(snapshot) = cached_stage_snapshot {
                    (snapshot, None)
                } else if let Some(snapshot) = direct_stage_snapshot {
                    (std::sync::Arc::clone(&snapshot), Some(snapshot))
                } else if let Some(usda) = stage_usda {
                    let snapshot = std::sync::Arc::new(build_stage_graph_snapshot_from_usda(&usda));
                    (std::sync::Arc::clone(&snapshot), Some(snapshot))
                } else {
                    let snapshot = std::sync::Arc::new(build_stage_graph_snapshot(&stage));
                    (snapshot, None)
                };
                let asset_registry = snapshot.asset_registry.clone();
                let (mut engine, compile_error) = match cached_engine {
                    Some(engine) => (engine, None),
                    None => match MarketLabGraphEngine::compile_from_canvas(snapshot.as_ref()) {
                        Ok(engine) => (engine, None),
                        Err(error) => {
                            return (
                                TimelineExecutionResult::default(),
                                Some(error.to_string()),
                                asset_registry,
                                None,
                                None,
                                built_snapshot,
                            );
                        }
                    },
                };
                let timeline_result = engine.execute_timeline(asset_vectors, timeline_len);
                let ui_snapshot = if compile_error.is_none() {
                    Some(H::build_graph_ui_snapshot(&timeline_result, &ui_build_input))
                } else {
                    None
                };
                (
                    timeline_result,
                    compile_error,
                    asset_registry,
                    Some(engine),
                    ui_snapshot,
                    built_snapshot,
                )
            })
            .await;
        let elapsed_ms = started.elapsed().as_millis() as u64;

        let _ = cx.update(|cx| {
            context_handle.update(cx, |workspace, _cx| {
                workspace.replace_computed_streams(timeline_result.streams.clone());
                workspace.replace_computed_token_streams(timeline_result.token_streams.clone());
                workspace.replace_asset_registry(asset_registry);
            });

            if let Some(entity) = this.upgrade() {
                let entity_for_resweep = entity.clone();
                entity.update(cx, |host, cx| {
                    if let Some(snapshot) = built_snapshot {
                        host.graph_engine_store_stage_snapshot(pipeline_revision, snapshot);
                    }
                    host.set_graph_engine_recompile_inflight(false);

                    let current_generation = host
                        .workspace_context()
                        .read(cx)
                        .engine_cache_generation();
                    if target_generation != current_generation {
                        host.set_graph_engine_recompile_pending(true);
                        begin_graph_engine_timeline_sweep(host, entity_for_resweep.clone(), cx);
                        return;
                    }

                    host.set_graph_engine_last_compiled_generation(target_generation);
                    host.set_graph_engine_last_swept_asset_epoch(target_asset_epoch);
                    host.set_graph_engine_last_compile_ms(elapsed_ms);
                    host.set_graph_engine_compile_error(compile_error.clone());
                    if let Some(error) = compile_error {
                        host.on_graph_engine_compile_failed(error, cx);
                    } else {
                        if let Some(engine) = compiled_engine {
                            host.store_cached_graph_engine(target_generation, engine);
                        }
                        host.apply_graph_engine_timeline_result(
                            timeline_result,
                            ui_snapshot.map(std::sync::Arc::new),
                            cx,
                        );
                    }

                    if host.graph_engine_recompile_pending() {
                        host.set_graph_engine_recompile_pending(false);
                        begin_graph_engine_timeline_sweep(host, entity_for_resweep, cx);
                    } else {
                        cx.notify();
                    }
                });
            }
        });
    })
    .detach();
}

/// Observe workspace-context edits and dispatch background graph recompilation.
pub fn install_graph_engine_invalidation_worker<H: GraphEngineInvalidationHost>(
    workspace: &Entity<WorkspaceContext>,
    cx: &mut Context<H>,
) {
    cx.observe(workspace, |host, workspace, cx| {
        if host.graph_engine_bootstrapping() {
            return;
        }
        if host.graph_engine_invalidation_deferred() {
            host.set_graph_engine_recompile_pending(true);
            return;
        }
        if workspace
            .read(cx)
            .is_engine_cache_dirty(host.graph_engine_last_compiled_generation())
        {
            host.spawn_graph_engine_recompile(cx);
        }
    })
    .detach();
}

/// Walk the passive USD stage and compile a graph snapshot from prim topology.
pub fn build_stage_graph_snapshot(stage: &ManagedUsdStage) -> StageGraphSnapshot {
    let mut snapshot = StageGraphSnapshot::default();
    let _ = stage.with_stage(|opened| {
        let root = Path::new("/")
            .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?;
        collect_stage_graph(stage, opened, &root, &mut snapshot)?;
        Ok(())
    });
    snapshot.path_bindings = build_path_binding_index(&snapshot.prims);
    snapshot.asset_registry = build_composed_asset_registry(stage, &snapshot.prims);
    snapshot
}

/// Parse composed USDA in memory and compile a graph snapshot (off UI thread).
pub fn build_stage_graph_snapshot_from_usda(usda: &str) -> StageGraphSnapshot {
    let context = WorkspaceContext::from_usda_text(usda).unwrap_or_default();
    build_stage_graph_snapshot(context.usd_stage())
}

pub fn build_path_binding_index(prims: &[StageGraphPrim]) -> PathBindingIndex {
    let mut ordered_prim_paths = Vec::new();
    let mut asset_slots = HashMap::new();
    for prim in prims {
        if prim.type_name != "FinancialAsset" {
            continue;
        }
        let slot = ordered_prim_paths.len();
        asset_slots.insert(prim.path.clone(), slot);
        ordered_prim_paths.push(prim.path.clone());
    }
    PathBindingIndex {
        asset_slots,
        ordered_prim_paths,
    }
}

fn resolved_prim_string(
    stage: &ManagedUsdStage,
    prim: &StageGraphPrim,
    attribute: &str,
) -> String {
    prim.attributes
        .get(attribute)
        .cloned()
        .filter(|value| !value.is_empty())
        .or_else(|| stage.field_string(&prim.path, attribute))
        .unwrap_or_default()
}

/// Build composed asset metadata for one `FinancialAsset` prim.
pub fn composed_asset_meta_from_prim(
    stage: &ManagedUsdStage,
    prim: &StageGraphPrim,
) -> ComposedAssetMeta {
    let symbol = resolved_prim_string(stage, prim, "inputs:symbol");
    let symbol = if symbol.is_empty() {
        prim.path
            .rsplit('/')
            .next()
            .unwrap_or(prim.path.as_str())
            .to_string()
    } else {
        symbol
    };
    let asset_class = resolved_prim_string(stage, prim, "inputs:asset_class");
    ComposedAssetMeta {
        symbol,
        asset_class: if asset_class.is_empty() {
            "Equity".to_string()
        } else {
            asset_class
        },
        category: resolved_prim_string(stage, prim, "inputs:category"),
        sub_category: resolved_prim_string(stage, prim, "inputs:sub_category"),
        is_active: stage.prim_active(&prim.path),
        sector: resolved_prim_string(stage, prim, "info:sector"),
        industry: resolved_prim_string(stage, prim, "info:industry"),
        market_cap_class: resolved_prim_string(stage, prim, "info:market_cap_class"),
        currency: resolved_prim_string(stage, prim, "info:currency"),
        country: resolved_prim_string(stage, prim, "info:country"),
        user_label: resolved_prim_string(stage, prim, USER_LABEL_ATTR),
    }
}

pub fn build_composed_asset_registry(
    stage: &ManagedUsdStage,
    prims: &[StageGraphPrim],
) -> HashMap<String, ComposedAssetMeta> {
    let mut registry = HashMap::new();
    for prim in prims {
        if prim.type_name != "FinancialAsset" {
            continue;
        }
        registry.insert(
            prim.path.clone(),
            composed_asset_meta_from_prim(stage, prim),
        );
    }
    registry
}

/// Back-compat alias for callers building a compile spec directly.
pub fn build_graph_compile_spec(stage: &ManagedUsdStage) -> StageGraphSnapshot {
    build_stage_graph_snapshot(stage)
}

fn collect_stage_graph(
    stage: &ManagedUsdStage,
    opened: &openusd::Stage,
    path: &Path,
    snapshot: &mut StageGraphSnapshot,
) -> Result<(), std::io::Error> {
    let children = opened
        .prim_children(path.clone())
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?;
    for child_name in children {
        let child_path = path
            .append_path(child_name.as_str())
            .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?;
        let path_str = child_path.to_string();
        if !stage.prim_active(&path_str) {
            continue;
        }

        if let Some(prim) = classify_prim(opened, &path_str) {
            snapshot.prims.push(prim);
        }

        for relationship in LINEAGE_RELATIONSHIPS {
            for target in stage.relationship_targets(&path_str, relationship) {
                snapshot.wires.push(GraphCompileWire {
                    source_prim_path: target,
                    target_prim_path: path_str.clone(),
                    relationship: relationship.to_string(),
                });
            }
        }

        collect_stage_graph(stage, opened, &child_path, snapshot)?;
    }
    Ok(())
}

fn is_schema_template_prim(path: &str) -> bool {
    matches!(
        path,
        "/FinancialAsset"
            | "/OtlOperator"
            | "/OtlTaUberSignal"
            | "/PortfolioIntegrator"
            | "/Typed"
            | "/Plugins"
            | "/Scope"
    )
}

fn prim_type_name(opened: &openusd::Stage, path_str: &str) -> Option<String> {
    opened
        .field::<String>(path_str, FieldKey::TypeName)
        .ok()
        .flatten()
        .map(|token| token.trim_matches('"').to_string())
        .filter(|name| !name.is_empty())
}

fn classify_type_name(type_name: &str) -> Option<String> {
    match type_name {
        "FinancialAsset" | "OtlOperator" | "OtlTaUberSignal" | "PortfolioIntegrator" => {
            Some(type_name.to_string())
        }
        _ => None,
    }
}

fn classify_prim(opened: &openusd::Stage, path_str: &str) -> Option<StageGraphPrim> {
    if is_schema_template_prim(path_str) {
        return None;
    }
    let type_name = prim_type_name(opened, path_str)
        .or_else(|| legacy_type_name_from_path(path_str))?;
    classify_type_name(&type_name)?;

    let mut attributes = HashMap::new();
    if let Ok(properties) = opened.prim_properties(Path::new(path_str).ok()?) {
        for property in properties {
            if property == "active" {
                continue;
            }
            let property_path = format!("{path_str}.{property}");
            if let Ok(Some(value)) = opened.field::<String>(property_path.as_str(), "default") {
                attributes.insert(property, value);
            } else if let Ok(Some(value)) =
                opened.field::<f64>(property_path.as_str(), "default")
            {
                attributes.insert(property, value.to_string());
            }
        }
    }

    if type_name == "FinancialAsset" && !attributes.contains_key("inputs:symbol") {
        if let Some(symbol) = path_str.rsplit('/').next() {
            attributes.insert("inputs:symbol".to_string(), symbol.to_string());
        }
    }

    Some(StageGraphPrim {
        path: path_str.to_string(),
        type_name: type_name.to_string(),
        attributes,
    })
}

fn legacy_type_name_from_path(path_str: &str) -> Option<String> {
    if path_str.starts_with("/assets/") {
        Some("FinancialAsset".to_string())
    } else if path_str.starts_with("/analytics/") {
        Some("OtlTaUberSignal".to_string())
    } else if path_str.starts_with("/portfolios/") {
        Some("PortfolioIntegrator".to_string())
    } else {
        None
    }
}

