//! Composition-stack layer resolution for the Details panel (§2.4).
//!
//! Models the LIVRPS precedence used by the finance workstation: **Session** overrides
//! **Signals** (graph canvas), which override **Schema** (metadata defaults).

use std::collections::HashMap;

use crate::blueprint::{finance_property_defaults, finance_property_fields};
use crate::stage_variants::format_variant_label;
use crate::usd_persistence::FinanceSessionContext;
use crate::types::type_id;

/// Composition layer identifiers (strongest → weakest).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FinanceCompositionLayer {
    Session,
    Signals,
    Schema,
}

impl FinanceCompositionLayer {
    pub fn label(self) -> &'static str {
        match self {
            Self::Session => "Session",
            Self::Signals => "Signals",
            Self::Schema => "Schema",
        }
    }

    pub fn source_label(self) -> &'static str {
        match self {
            Self::Session => "session.usda",
            Self::Signals => "signals.usda",
            Self::Schema => "schema",
        }
    }
}

/// One contribution on the composition stack for a property.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FinanceLayerContribution {
    pub layer: FinanceCompositionLayer,
    pub raw_value: String,
    pub display_value: String,
}

/// Resolved property with active layer and hidden lower layers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FinancePropertyLayerResolution {
    pub property_id: String,
    pub property_label: String,
    pub active_layer: FinanceCompositionLayer,
    pub active_value: String,
    pub active_display: String,
    /// True when a stronger layer hides a weaker contribution.
    pub has_layer_override: bool,
    pub stack: Vec<FinanceLayerContribution>,
    pub overridden_layers: Vec<FinanceLayerContribution>,
}

fn format_layer_value(property_id: &str, raw: &str) -> String {
    if property_id == "allocation_id" {
        format_variant_label(raw)
    } else {
        raw.to_string()
    }
}

fn session_layer_value(
    property_id: &str,
    node_id: &str,
    session_variant_overrides: &HashMap<String, String>,
    session_ctx: Option<&FinanceSessionContext<'_>>,
) -> Option<String> {
    if property_id == "allocation_id" {
        return session_variant_overrides.get(node_id).cloned();
    }
    let Some(ctx) = session_ctx else {
        return None;
    };
    let prim_path = ctx
        .resolved_prim_paths
        .get(node_id)
        .map(String::as_str)
        .or_else(|| {
            if property_id == "prim_path" {
                ctx.resolved_prim_paths.get(node_id).map(String::as_str)
            } else {
                None
            }
        })?;
    let attrs = ctx.opinions.get(prim_path)?;
    let usd_key = graph_property_to_usd_attr(property_id);
    attrs.get(usd_key).cloned()
}

fn graph_property_to_usd_attr(property_id: &str) -> &str {
    match property_id {
        "symbol" => "inputs:symbol",
        "asset_class" => "inputs:asset_class",
        "csv_path" => "inputs:csv_path",
        "prim_path" => "inputs:prim_path",
        "category" => "inputs:category",
        "sub_category" => "inputs:sub_category",
        "exchange_mic" => "inputs:exchange_mic",
        "allocation_id" => "inputs:id",
        "display_name" => "info:user_label",
        other => other,
    }
}

/// Build layer-resolution rows for every inspector field on a finance node.
pub fn finance_property_layer_resolutions(
    definition_id: &str,
    node_id: &str,
    graph_properties: &HashMap<String, String>,
    session_variant_overrides: &HashMap<String, String>,
) -> Vec<FinancePropertyLayerResolution> {
    finance_property_layer_resolutions_with_session(
        definition_id,
        node_id,
        graph_properties,
        session_variant_overrides,
        None,
    )
}

/// Same as [`finance_property_layer_resolutions`] with optional session.usda opinions.
pub fn finance_property_layer_resolutions_with_session(
    definition_id: &str,
    node_id: &str,
    graph_properties: &HashMap<String, String>,
    session_variant_overrides: &HashMap<String, String>,
    session_ctx: Option<&FinanceSessionContext<'_>>,
) -> Vec<FinancePropertyLayerResolution> {
    let schema_defaults = finance_property_defaults(definition_id);
    let mut rows: Vec<FinancePropertyLayerResolution> = finance_property_fields(definition_id)
        .into_iter()
        .map(|field| {
            let schema_value = schema_defaults
                .get(&field.id)
                .cloned()
                .unwrap_or_default();
            let signals_value = graph_properties
                .get(&field.id)
                .cloned()
                .unwrap_or_else(|| schema_value.clone());
            let session_value = session_layer_value(
                &field.id,
                node_id,
                session_variant_overrides,
                session_ctx,
            );

            let mut stack = Vec::new();
            if let Some(session) = session_value.clone() {
                stack.push(FinanceLayerContribution {
                    layer: FinanceCompositionLayer::Session,
                    display_value: format_layer_value(&field.id, &session),
                    raw_value: session,
                });
            }
            stack.push(FinanceLayerContribution {
                layer: FinanceCompositionLayer::Signals,
                display_value: format_layer_value(&field.id, &signals_value),
                raw_value: signals_value.clone(),
            });
            if !schema_value.is_empty() {
                stack.push(FinanceLayerContribution {
                    layer: FinanceCompositionLayer::Schema,
                    display_value: format_layer_value(&field.id, &schema_value),
                    raw_value: schema_value,
                });
            }

            let (active_layer, active_value, active_display) =
                if let Some(session) = session_value.clone() {
                    (
                        FinanceCompositionLayer::Session,
                        session.clone(),
                        format_layer_value(&field.id, &session),
                    )
                } else {
                    (
                        FinanceCompositionLayer::Signals,
                        signals_value.clone(),
                        format_layer_value(&field.id, &signals_value),
                    )
                };

            let overridden_layers: Vec<FinanceLayerContribution> = stack
                .iter()
                .filter(|layer| layer.layer != active_layer && layer.raw_value != active_value)
                .cloned()
                .collect();
            let has_layer_override = !overridden_layers.is_empty()
                || (active_layer == FinanceCompositionLayer::Signals
                    && signals_value != schema_defaults.get(&field.id).cloned().unwrap_or_default()
                    && !schema_defaults.get(&field.id).map(String::as_str).unwrap_or("").is_empty());

            FinancePropertyLayerResolution {
                property_id: field.id,
                property_label: field.label,
                active_layer,
                active_value,
                active_display,
                has_layer_override,
                stack,
                overridden_layers,
            }
        })
        .collect();

    if definition_id == type_id::FINANCIAL_ASSET {
        if graph_properties
            .get("prim_path")
            .map(String::as_str)
            .unwrap_or("")
            .is_empty()
        {
            if let Some(ctx) = session_ctx {
                if let Some(resolved) = ctx.resolved_prim_paths.get(node_id) {
                    rows.push(computed_prim_path_row(resolved));
                }
            }
        }
    }

    rows
}

fn computed_prim_path_row(resolved: &str) -> FinancePropertyLayerResolution {
    FinancePropertyLayerResolution {
        property_id: "resolved_prim_path".to_string(),
        property_label: "Resolved prim path".to_string(),
        active_layer: FinanceCompositionLayer::Schema,
        active_value: resolved.to_string(),
        active_display: resolved.to_string(),
        has_layer_override: false,
        stack: vec![FinanceLayerContribution {
            layer: FinanceCompositionLayer::Schema,
            raw_value: resolved.to_string(),
            display_value: resolved.to_string(),
        }],
        overridden_layers: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::type_id;

    #[test]
    fn session_allocation_override_masks_signals() {
        let graph = HashMap::from([(
            "allocation_id".to_string(),
            "Allocation::HierarchicalRiskParity".to_string(),
        )]);
        let mut session = HashMap::from([(
            "fund".to_string(),
            "Allocation::MeanVariance".to_string(),
        )]);
        let rows = finance_property_layer_resolutions(
            type_id::PORTFOLIO_INTEGRATOR,
            "fund",
            &graph,
            &session,
        );
        let allocation = rows
            .iter()
            .find(|row| row.property_id == "allocation_id")
            .expect("allocation row");
        assert_eq!(allocation.active_layer, FinanceCompositionLayer::Session);
        assert!(allocation.has_layer_override);
        assert_eq!(allocation.overridden_layers.len(), 2);
        assert_eq!(
            allocation.overridden_layers[0].layer,
            FinanceCompositionLayer::Signals
        );

        session.clear();
        let rows = finance_property_layer_resolutions(
            type_id::PORTFOLIO_INTEGRATOR,
            "fund",
            &graph,
            &session,
        );
        let allocation = rows
            .iter()
            .find(|row| row.property_id == "allocation_id")
            .expect("allocation row");
        assert_eq!(allocation.active_layer, FinanceCompositionLayer::Signals);
        assert!(!allocation.has_layer_override);
        assert!(allocation.overridden_layers.is_empty());
    }

    #[test]
    fn strategy_channel_marks_schema_when_graph_differs() {
        let graph = HashMap::from([("aggression".to_string(), "0.80".to_string())]);
        let rows = finance_property_layer_resolutions(
            type_id::TA_TREND,
            "ta1",
            &graph,
            &HashMap::new(),
        );
        let aggression = rows
            .iter()
            .find(|row| row.property_id == "aggression")
            .expect("aggression row");
        assert_eq!(aggression.active_layer, FinanceCompositionLayer::Signals);
        assert!(aggression.has_layer_override);
        assert!(aggression
            .overridden_layers
            .iter()
            .any(|layer| layer.layer == FinanceCompositionLayer::Schema));
    }
}
