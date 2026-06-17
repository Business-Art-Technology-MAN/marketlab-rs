//! Logical strategy topology compiled from `inputs:sources` linkage.

use std::collections::{HashMap, HashSet};
use std::io;

use openusd::sdf::schema::FieldKey;
use openusd::sdf::Path;
use pulsar_marketlab_core::{prim_display_label, ComposedAssetMeta, USER_LABEL_ATTR};

use super::context::ManagedUsdStage;

#[derive(Clone, Debug)]
pub struct LogicalTreeNode {
    pub prim_path: String,
    pub display_label: String,
    pub children: Vec<LogicalTreeNode>,
}

/// True when the composed stage contains at least one active `PortfolioIntegrator`.
pub fn stage_has_portfolio_integrators(stage: &ManagedUsdStage) -> bool {
    enumerate_stage_prims(stage)
        .into_iter()
        .any(|(_, type_name)| type_name == "PortfolioIntegrator")
}

pub fn compile_logical_strategy_tree(
    stage: &ManagedUsdStage,
    asset_registry: &HashMap<String, ComposedAssetMeta>,
) -> Vec<LogicalTreeNode> {
    let indexed_prims = enumerate_stage_prims(stage);
    let portfolio_roots: Vec<String> = indexed_prims
        .iter()
        .filter(|(_, type_name)| type_name == "PortfolioIntegrator")
        .map(|(path, _)| path.clone())
        .collect();

    if !portfolio_roots.is_empty() {
        let referenced = portfolio_paths_referenced_as_sources(stage, &portfolio_roots);
        let mut visited = HashSet::new();
        return portfolio_roots
            .into_iter()
            .filter(|path| !referenced.contains(path))
            .map(|path| build_subtree(stage, asset_registry, &path, &mut visited))
            .collect();
    }

    indexed_prims
        .into_iter()
        .filter(|(_, type_name)| {
            type_name == "FinancialAsset"
                || type_name == "OtlTaUberSignal"
                || type_name == "OtlOperator"
        })
        .map(|(path, _)| LogicalTreeNode {
            prim_path: path.clone(),
            display_label: node_label(stage, asset_registry, &path),
            children: Vec::new(),
        })
        .collect()
}

fn enumerate_stage_prims(stage: &ManagedUsdStage) -> Vec<(String, String)> {
    let mut prims = Vec::new();
    let _ = stage.with_stage(|opened| {
        let root = Path::new("/")
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        collect_prim_types(stage, opened, &root, &mut prims)?;
        Ok(())
    });
    prims
}

fn collect_prim_types(
    stage: &ManagedUsdStage,
    opened: &openusd::Stage,
    path: &Path,
    prims: &mut Vec<(String, String)>,
) -> Result<(), io::Error> {
    let children = opened
        .prim_children(path.clone())
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    for child_name in children {
        let child_path = path
            .append_path(child_name.as_str())
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        let path_str = child_path.to_string();
        if !stage.prim_active(&path_str) {
            continue;
        }
        if let Some(type_name) = classify_stage_prim_type(opened, &path_str) {
            prims.push((path_str.clone(), type_name));
        }
        collect_prim_types(stage, opened, &child_path, prims)?;
    }
    Ok(())
}

/// Portfolios referenced as `inputs:sources` by another portfolio are not top-level roots.
fn portfolio_paths_referenced_as_sources(
    stage: &ManagedUsdStage,
    portfolio_paths: &[String],
) -> HashSet<String> {
    let portfolio_set: HashSet<String> = portfolio_paths.iter().cloned().collect();
    let mut referenced = HashSet::new();
    for port_path in portfolio_paths {
        for source in stage.relationship_targets(port_path, "inputs:sources") {
            if portfolio_set.contains(&source) {
                referenced.insert(source);
            }
        }
    }
    referenced
}

fn build_subtree(
    stage: &ManagedUsdStage,
    asset_registry: &HashMap<String, ComposedAssetMeta>,
    prim_path: &str,
    visited: &mut HashSet<String>,
) -> LogicalTreeNode {
    let display_label = node_label(stage, asset_registry, prim_path);
    if !visited.insert(prim_path.to_string()) {
        return LogicalTreeNode {
            prim_path: prim_path.to_string(),
            display_label,
            children: Vec::new(),
        };
    }

    let children = stage
        .relationship_targets(prim_path, "inputs:sources")
        .into_iter()
        .map(|source| build_subtree(stage, asset_registry, &source, visited))
        .collect();

    visited.remove(prim_path);

    LogicalTreeNode {
        prim_path: prim_path.to_string(),
        display_label,
        children,
    }
}

fn classify_stage_prim_type(opened: &openusd::Stage, path_str: &str) -> Option<String> {
    if matches!(
        path_str,
        "/FinancialAsset"
            | "/OtlOperator"
            | "/OtlTaUberSignal"
            | "/PortfolioIntegrator"
            | "/Typed"
            | "/Plugins"
            | "/Scope"
    ) {
        return None;
    }
    let type_name = opened
        .field::<String>(path_str, FieldKey::TypeName)
        .ok()
        .flatten()
        .map(|token| token.trim_matches('"').to_string())
        .filter(|name| !name.is_empty())
        .or_else(|| legacy_type_name_from_path(path_str))?;
    match type_name.as_str() {
        "FinancialAsset" | "OtlOperator" | "OtlTaUberSignal" | "PortfolioIntegrator" => {
            Some(type_name)
        }
        _ => None,
    }
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

fn node_label(
    stage: &ManagedUsdStage,
    asset_registry: &HashMap<String, ComposedAssetMeta>,
    prim_path: &str,
) -> String {
    let leaf = prim_path.rsplit('/').next().unwrap_or(prim_path);
    let stage_user_label = stage.field_string(prim_path, USER_LABEL_ATTR);
    let user_label = asset_registry
        .get(prim_path)
        .map(|meta| meta.user_label.as_str())
        .filter(|label| !label.trim().is_empty())
        .or_else(|| stage_user_label.as_deref());
    prim_display_label(leaf, user_label)
}
