//! Stage-tree variant tokens (allocation methods, etc.) for Hydra profile switching.

use std::collections::HashMap;

use graphy::NodeInstance;

use crate::types::{type_id, PORTFOLIO_ALLOCATION_TOKENS};

/// One selectable variant in the stage tree Variant column.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StageVariantOption {
    pub token: String,
    pub label: String,
}

/// List switchable variants for a finance node definition id (empty if read-only).
pub fn finance_stage_variant_options(definition_id: &str) -> Vec<StageVariantOption> {
    if definition_id == type_id::PORTFOLIO_INTEGRATOR {
        return PORTFOLIO_ALLOCATION_TOKENS
            .iter()
            .map(|token| StageVariantOption {
                label: format_variant_label(token),
                token: (*token).to_string(),
            })
            .collect();
    }
    Vec::new()
}

/// Default variant token from graph node properties (before UI overrides).
pub fn default_variant_token(node: &NodeInstance) -> Option<String> {
    if node.node_type == type_id::PORTFOLIO_INTEGRATOR {
        return node
            .properties
            .get("allocation_id")
            .and_then(|value| match value {
                graphy::JsonValue::String(text) if !text.trim().is_empty() => {
                    Some(text.trim().to_string())
                }
                _ => None,
            })
            .or_else(|| Some(PORTFOLIO_ALLOCATION_TOKENS[0].to_string()));
    }
    None
}

/// Human-readable `[Label]` for a variant token.
pub fn format_variant_label(token: &str) -> String {
    let short = token
        .rsplit("::")
        .next()
        .unwrap_or(token)
        .replace('_', " ");
    format!("[{short}]")
}

/// Active token: stage-tree override wins over compiled graph default.
pub fn resolve_variant_token(
    node_id: &str,
    defaults: &HashMap<String, String>,
    overrides: &HashMap<String, String>,
) -> Option<String> {
    overrides
        .get(node_id)
        .cloned()
        .or_else(|| defaults.get(node_id).cloned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use graphy::{JsonValue, NodeInstance, Position};

    #[test]
    fn portfolio_has_three_allocation_variants() {
        let options = finance_stage_variant_options(type_id::PORTFOLIO_INTEGRATOR);
        assert_eq!(options.len(), 3);
        assert!(options.iter().any(|o| o.token.contains("EqualWeight")));
    }

    #[test]
    fn override_wins_over_default() {
        let mut defaults = HashMap::from([("p1".to_string(), "Allocation::EqualWeight".to_string())]);
        let mut overrides = HashMap::from([(
            "p1".to_string(),
            "Allocation::MeanVariance".to_string(),
        )]);
        assert_eq!(
            resolve_variant_token("p1", &defaults, &overrides).as_deref(),
            Some("Allocation::MeanVariance")
        );
        overrides.remove("p1");
        assert_eq!(
            resolve_variant_token("p1", &defaults, &overrides).as_deref(),
            Some("Allocation::EqualWeight")
        );
        defaults.clear();
        assert!(resolve_variant_token("p1", &defaults, &overrides).is_none());
    }

    #[test]
    fn reads_allocation_from_node_properties() {
        let mut node = NodeInstance::new(
            "fund",
            type_id::PORTFOLIO_INTEGRATOR,
            Position::new(0.0, 0.0),
        );
        node.properties.insert(
            "allocation_id".to_string(),
            JsonValue::String("Allocation::MeanVariance".to_string()),
        );
        assert_eq!(
            default_variant_token(&node).as_deref(),
            Some("Allocation::MeanVariance")
        );
    }
}
