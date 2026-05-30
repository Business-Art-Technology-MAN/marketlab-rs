//! Unified MarketLab scene graph path conventions and prim classification.

use openusd::sdf::schema::FieldKey;
use openusd::sdf::{Specifier, Value};
use openusd::Stage;

use super::MarketStagePathError;

/// Root scope for all operational execution prims.
pub const MARKETLAB_ROOT: &str = "/MarketLab";

/// Default prim token written into composed USDA metadata.
pub const MARKETLAB_DEFAULT_PRIM: &str = "MarketLab";

/// Schema template paths that must never appear in the stage composer tree.
pub const SCHEMA_TEMPLATE_PRIM_PATHS: &[&str] = &[
    "/FinancialAsset",
    "/OtlOperator",
    "/PortfolioIntegrator",
    "/Typed",
    "/Plugins",
    "/Scope",
];

/// Build a child prim path directly under [`MARKETLAB_ROOT`].
pub fn marketlab_leaf_path(leaf: &str) -> Result<String, MarketStagePathError> {
    nested_prim_path(MARKETLAB_ROOT, leaf)
}

/// Append a sanitized leaf segment to an existing absolute prim path.
pub fn nested_prim_path(parent: &str, leaf: &str) -> Result<String, MarketStagePathError> {
    let leaf = leaf.trim().replace(' ', "_");
    if leaf.is_empty() || leaf.contains('/') {
        return Err(MarketStagePathError::InvalidPath);
    }
    let path = format!("{parent}/{leaf}");
    super::validate_stage_path(&path)?;
    Ok(path)
}

/// Returns true when a path points at a schema template rather than an instance prim.
pub fn is_schema_template_prim(path: &str) -> bool {
    if SCHEMA_TEMPLATE_PRIM_PATHS.contains(&path) {
        return true;
    }
    let segments: Vec<&str> = path.split('/').filter(|segment| !segment.is_empty()).collect();
    segments.len() == 1
        && matches!(
            segments[0],
            "FinancialAsset" | "OtlOperator" | "PortfolioIntegrator" | "Typed" | "Plugins"
        )
}

/// Skip abstract `class` specs and schema templates when building UI trees.
pub fn should_show_prim_in_stage_tree(stage: &Stage, path: &str) -> bool {
    if is_schema_template_prim(path) {
        return false;
    }
    !prim_is_class_spec(stage, path)
}

pub fn prim_is_class_spec(stage: &Stage, path: &str) -> bool {
    matches!(
        stage
            .field::<Value>(path, FieldKey::Specifier)
            .ok()
            .flatten(),
        Some(Value::Specifier(Specifier::Class))
    )
}

pub fn prim_type_name(stage: &Stage, path: &str) -> Option<String> {
    stage
        .field::<String>(path, FieldKey::TypeName)
        .ok()
        .flatten()
        .map(|token| token.trim_matches('"').to_string())
        .filter(|name| !name.is_empty())
}

/// Classify an operational prim using composed `typeName` metadata.
pub fn classify_type_name(type_name: &str) -> Option<ExecutablePrimKind> {
    match type_name {
        "FinancialAsset" => Some(ExecutablePrimKind::FinancialAsset),
        "OtlOperator" => Some(ExecutablePrimKind::OtlOperator),
        "PortfolioIntegrator" => Some(ExecutablePrimKind::PortfolioIntegrator),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutablePrimKind {
    FinancialAsset,
    OtlOperator,
    PortfolioIntegrator,
}

impl ExecutablePrimKind {
    pub fn schema_type_token(self) -> &'static str {
        match self {
            Self::FinancialAsset => "FinancialAsset",
            Self::OtlOperator => "OtlOperator",
            Self::PortfolioIntegrator => "PortfolioIntegrator",
        }
    }
}

/// Legacy flat paths (`/assets`, `/analytics`, `/portfolios`) and nested MarketLab paths.
pub fn is_legacy_bucket_path(path: &str) -> bool {
    path == "/assets" || path == "/analytics" || path == "/portfolios"
}

pub fn is_operational_instance_path(path: &str) -> bool {
    if is_schema_template_prim(path) || is_legacy_bucket_path(path) || path == MARKETLAB_ROOT {
        return false;
    }
    path.starts_with("/assets/")
        || path.starts_with("/analytics/")
        || path.starts_with("/portfolios/")
        || path.starts_with(&format!("{MARKETLAB_ROOT}/"))
}
