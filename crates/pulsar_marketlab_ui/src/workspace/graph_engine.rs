//! Background graph-engine invalidation worker for the workstation layout.

use std::collections::HashMap;

use gpui::*;
use openusd::sdf::schema::FieldKey;
use openusd::sdf::Path;
use pulsar_marketlab_core::{
    ComputedAttributeStream, GraphCompileWire, MarketLabGraphEngine, StageGraphPrim,
    StageGraphSnapshot,
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

    fn graph_engine_asset_vectors(&self) -> HashMap<String, Vec<f64>>;

    fn graph_engine_timeline_len(&self) -> usize;

    fn graph_engine_last_compiled_generation(&self) -> u64;

    fn set_graph_engine_last_compiled_generation(&mut self, generation: u64);

    fn graph_engine_recompile_inflight(&self) -> bool;

    fn set_graph_engine_recompile_inflight(&mut self, inflight: bool);

    fn graph_engine_recompile_pending(&self) -> bool;

    fn set_graph_engine_recompile_pending(&mut self, pending: bool);

    fn apply_graph_engine_streams(
        &mut self,
        streams: Vec<ComputedAttributeStream>,
        cx: &mut Context<Self>,
    );

    fn spawn_graph_engine_recompile(&mut self, cx: &mut Context<Self>) {
        begin_graph_engine_timeline_sweep(self, cx.entity(), cx);
    }
}

/// Dispatch a background graph-engine timeline sweep and bounce results to UI state.
pub fn spawn_graph_engine_timeline_sweep<H: GraphEngineInvalidationHost>(
    view: Entity<H>,
    cx: &mut Context<H>,
) {
    let entity = view.clone();
    view.update(cx, |host, cx| {
        begin_graph_engine_timeline_sweep(host, entity, cx);
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
    if generation == host.graph_engine_last_compiled_generation() {
        return;
    }
    if host.graph_engine_recompile_inflight() {
        host.set_graph_engine_recompile_pending(true);
        return;
    }

    host.set_graph_engine_recompile_inflight(true);
    let asset_vectors = host.graph_engine_asset_vectors();
    let timeline_len = host.graph_engine_timeline_len();
    let stage = context_handle.read(cx).usd_stage().clone();
    let target_generation = generation;

    cx.spawn(async move |this, cx| {
        let computed_streams = cx
            .background_executor()
            .spawn(async move {
                let snapshot = build_stage_graph_snapshot(&stage);
                MarketLabGraphEngine::compile_from_stage(&snapshot)
                    .ok()
                    .map(|engine| engine.execute_timeline(&asset_vectors, timeline_len))
                    .unwrap_or_default()
            })
            .await;

        let _ = cx.update(|cx| {
            context_handle.update(cx, |workspace, cx| {
                workspace.replace_computed_streams(computed_streams.clone());
                cx.notify();
            });

            if let Some(entity) = this.upgrade() {
                let entity_for_resweep = entity.clone();
                entity.update(cx, |host, cx| {
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
                    host.apply_graph_engine_streams(computed_streams, cx);

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
    snapshot
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
        "FinancialAsset" | "OtlOperator" | "PortfolioIntegrator" => Some(type_name.to_string()),
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
        Some("OtlOperator".to_string())
    } else if path_str.starts_with("/portfolios/") {
        Some("PortfolioIntegrator".to_string())
    } else {
        None
    }
}
