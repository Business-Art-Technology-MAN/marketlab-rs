//! Graphy [`NodeMetadata`] definitions for MarketLab finance nodes.

use std::collections::HashMap;

use graphy::{
    NodeMetadata, NodeTypes, ParamInfo, PropertySchema, PropertyValue, ReflectedType, TypeInfo,
};
use pulsar_marketlab_core::TaArchetype;

use crate::blueprint::FINANCE_SIGNAL_TYPE;
use crate::types::{category, type_id, PORTFOLIO_ALLOCATION_TOKENS};

/// Unified strategy channels rendered on analytics node faces (0..1).
fn strategy_channel_schema() -> Vec<PropertySchema> {
    vec![
        PropertySchema::new(
            "aggression",
            "Aggression",
            ReflectedType::parse_str("f64"),
        )
        .with_default(PropertyValue::Float(0.5))
        .with_tooltip("Execution velocity and order-impact limits"),
        PropertySchema::new("decay", "Decay", ReflectedType::parse_str("f64"))
            .with_default(PropertyValue::Float(0.35))
            .with_tooltip("Data attenuation and historical memory decay"),
        PropertySchema::new(
            "elasticity",
            "Elasticity",
            ReflectedType::parse_str("f64"),
        )
        .with_default(PropertyValue::Float(0.55))
        .with_tooltip("Return-to-base pacing after volatility spikes"),
    ]
}

fn signal_series_param(name: &str) -> ParamInfo {
    ParamInfo::new(name, FINANCE_SIGNAL_TYPE)
}

fn financial_asset_metadata() -> NodeMetadata {
    NodeMetadata::new(
        type_id::FINANCIAL_ASSET,
        NodeTypes::pure,
        category::UNIVERSE,
    )
    .with_return_type(TypeInfo::new(FINANCE_SIGNAL_TYPE))
    .with_property_schema(vec![
        PropertySchema::new("symbol", "Symbol", ReflectedType::parse_str("String"))
            .with_default(PropertyValue::String("SPY".into())),
        PropertySchema::new("csv_path", "CSV path", ReflectedType::parse_str("String"))
            .with_tooltip(
                "Optional. Leave empty to auto-load crates/pulsar_marketlab/data/{symbol}.csv",
            ),
        PropertySchema::new("prim_path", "Stage path", ReflectedType::parse_str("String"))
            .with_tooltip("Absolute USD prim path, e.g. /MarketLab/Universe/SPY"),
        PropertySchema::new("asset_class", "Asset class", ReflectedType::parse_str("String"))
            .with_default(PropertyValue::String("Equity".into())),
    ])
    .with_doc("Financial Asset")
    .with_version(1)
}

fn otl_operator_metadata() -> NodeMetadata {
    NodeMetadata::new(
        type_id::OTL_OPERATOR,
        NodeTypes::pure,
        category::ANALYTICS,
    )
    .with_params(vec![signal_series_param("underlying")])
    .with_return_type(TypeInfo::new(FINANCE_SIGNAL_TYPE))
    .with_property_schema({
        let mut schema = vec![
            PropertySchema::new(
                "script_src",
                "OTL source",
                ReflectedType::parse_str("String"),
            ),
            PropertySchema::new(
                "script_compiled_path",
                "Compiled OTL path",
                ReflectedType::parse_str("String"),
            ),
        ];
        schema.extend(strategy_channel_schema());
        schema
    })
    .with_doc("OTL Operator")
    .with_version(1)
}

fn ta_uber_metadata(type_id: &'static str, archetype: TaArchetype) -> NodeMetadata {
    NodeMetadata::new(type_id, NodeTypes::pure, category::ANALYTICS)
        .with_params(vec![signal_series_param("source_stream")])
        .with_return_type(TypeInfo::new(FINANCE_SIGNAL_TYPE))
        .with_property_schema(vec![
            PropertySchema::new(
                "archetype",
                "Archetype",
                ReflectedType::parse_str("String"),
            )
            .with_default(PropertyValue::String(
                archetype.as_token().to_string(),
            )),
            PropertySchema::new(
                "algorithm",
                "Algorithm",
                ReflectedType::parse_str("String"),
            )
            .with_default(PropertyValue::String(
                archetype.default_algorithm().to_string(),
            )),
            PropertySchema::new("period", "Period", ReflectedType::parse_str("u32"))
                .with_default(PropertyValue::Int(archetype.default_period() as i64)),
            PropertySchema::new("signal_period", "Signal period", ReflectedType::parse_str("u32"))
                .with_default(PropertyValue::Int(9)),
            PropertySchema::new("multiplier", "Multiplier", ReflectedType::parse_str("f64"))
                .with_default(PropertyValue::Float(2.0)),
            PropertySchema::new(
                "annualization",
                "Annualization",
                ReflectedType::parse_str("f64"),
            )
            .with_default(PropertyValue::Float(252.0)),
        ]
        .into_iter()
        .chain(strategy_channel_schema())
        .collect())
        .with_doc(archetype.display_name())
        .with_version(1)
}

fn portfolio_integrator_metadata() -> NodeMetadata {
    let signal_inputs: Vec<ParamInfo> = (0..8)
        .map(|idx| signal_series_param(&format!("signal_{idx}")))
        .collect();

    NodeMetadata::new(
        type_id::PORTFOLIO_INTEGRATOR,
        NodeTypes::pure,
        category::PORTFOLIOS,
    )
    .with_params(signal_inputs)
    .with_return_type(TypeInfo::new(FINANCE_SIGNAL_TYPE))
    .with_property_schema(vec![
        PropertySchema::new("name", "Fund name", ReflectedType::parse_str("String"))
            .with_default(PropertyValue::String("Fund".into())),
        PropertySchema::new(
            "allocation_id",
            "Allocation method",
            ReflectedType::parse_str("String"),
        )
        .with_default(PropertyValue::String(
            PORTFOLIO_ALLOCATION_TOKENS[0].to_string(),
        ))
        .with_tooltip(PORTFOLIO_ALLOCATION_TOKENS.join(", ")),
        PropertySchema::new(
            "initial_capital",
            "Initial capital",
            ReflectedType::parse_str("f64"),
        )
        .with_default(PropertyValue::Float(10_000_000.0)),
        PropertySchema::new(
            "rebalance_frequency",
            "Rebalance",
            ReflectedType::parse_str("String"),
        )
        .with_default(PropertyValue::String("monthly".into())),
    ])
    .with_doc("Portfolio Integrator")
    .with_version(1)
}

/// Build the full finance node catalog keyed by Graphy `node_type` id.
pub fn finance_node_catalog() -> HashMap<String, NodeMetadata> {
    let mut catalog = HashMap::new();

    let entries: Vec<NodeMetadata> = vec![
        financial_asset_metadata(),
        otl_operator_metadata(),
        ta_uber_metadata(type_id::TA_TREND, TaArchetype::Trend),
        ta_uber_metadata(type_id::TA_VOLATILITY, TaArchetype::Volatility),
        ta_uber_metadata(type_id::TA_OSCILLATOR, TaArchetype::Oscillator),
        ta_uber_metadata(type_id::TA_CHANNEL, TaArchetype::Channel),
        portfolio_integrator_metadata(),
    ];

    for meta in entries {
        catalog.insert(meta.name.clone(), meta);
    }

    catalog
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_registers_all_finance_archetypes() {
        let catalog = finance_node_catalog();
        assert!(catalog.contains_key(type_id::FINANCIAL_ASSET));
        assert!(catalog.contains_key(type_id::OTL_OPERATOR));
        assert!(catalog.contains_key(type_id::TA_OSCILLATOR));
        assert!(catalog.contains_key(type_id::PORTFOLIO_INTEGRATOR));
    }

    #[test]
    fn portfolio_metadata_exposes_allocation_property() {
        let catalog = finance_node_catalog();
        let meta = catalog
            .get(type_id::PORTFOLIO_INTEGRATOR)
            .expect("portfolio");
        assert!(
            meta.property_schema
                .iter()
                .any(|field| field.id == "allocation_id")
        );
    }
}
