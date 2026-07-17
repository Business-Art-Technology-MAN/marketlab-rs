//! Graphy [`NodeMetadata`] definitions for MarketLab finance nodes.

use std::collections::HashMap;

use graphy::{
    NodeMetadata, NodeTypes, ParamInfo, PropertySchema, PropertyValue, ReflectedType, TypeInfo,
};
use pulsar_marketlab_core::TaArchetype;

use crate::blueprint::FINANCE_SIGNAL_TYPE;
use crate::portfolio_pins::portfolio_signal_pin_id;
use crate::series_pins::{performance_series_pin_id, PERFORMANCE_BENCHMARK_PIN};
use crate::types::{category, type_id, PORTFOLIO_ALLOCATION_TOKENS};

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

fn financial_return_asset_metadata() -> NodeMetadata {
    NodeMetadata::new(
        type_id::FINANCIAL_RETURN_ASSET,
        NodeTypes::pure,
        category::UNIVERSE,
    )
    .with_return_type(TypeInfo::new(FINANCE_SIGNAL_TYPE))
    .with_property_schema(vec![
        PropertySchema::new("symbol", "Symbol", ReflectedType::parse_str("String"))
            .with_tooltip("Optional — leave empty to use the CSV column header (e.g. Winton)"),
        PropertySchema::new("csv_path", "CSV path", ReflectedType::parse_str("String"))
            .with_tooltip("Required. `Date,<Name>` header row; second column = simple returns"),
        PropertySchema::new("prim_path", "Stage path", ReflectedType::parse_str("String"))
            .with_tooltip("Absolute USD prim path, e.g. /MarketLab/Universe/Winton"),
        PropertySchema::new("asset_class", "Asset class", ReflectedType::parse_str("String"))
            .with_default(PropertyValue::String("Alternative".into())),
    ])
    .with_doc("Return Series Asset")
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
    .with_property_schema(vec![
        PropertySchema::new(
            "script_src",
            "OTL source",
            ReflectedType::parse_str("String"),
        )
        .with_default(PropertyValue::String(
            "ta::spread_sign(ta::sma(input, 10), ta::sma(input, 50))".into(),
        )),
        PropertySchema::new(
            "script_compiled_path",
            "Compiled OTL path",
            ReflectedType::parse_str("String"),
        ),
    ])
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
                .with_default(PropertyValue::Int(archetype.default_period() as i64))
                .with_tooltip("Fast MA lookback for crossover signals (shorter of the two MA periods)"),
            PropertySchema::new("signal_period", "Signal period", ReflectedType::parse_str("u32"))
                .with_default(PropertyValue::Int(archetype.default_signal_period() as i64))
                .with_tooltip("Slow MA lookback for crossover signals (longer of the two MA periods)"),
            PropertySchema::new("multiplier", "Multiplier", ReflectedType::parse_str("f64"))
                .with_default(PropertyValue::Float(2.0)),
            PropertySchema::new(
                "annualization",
                "Annualization",
                ReflectedType::parse_str("f64"),
            )
            .with_default(PropertyValue::Float(252.0)),
        ])
        .with_doc(archetype.display_name())
        .with_version(1)
}

fn portfolio_integrator_metadata() -> NodeMetadata {
    let signal_inputs: Vec<ParamInfo> =
        vec![signal_series_param(&portfolio_signal_pin_id(0))];

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

fn performance_analytics_metadata() -> NodeMetadata {
    NodeMetadata::new(
        type_id::PERFORMANCE_ANALYTICS,
        NodeTypes::pure,
        category::REPORTING,
    )
    .with_params(vec![
        signal_series_param(&performance_series_pin_id(0)),
        signal_series_param(PERFORMANCE_BENCHMARK_PIN),
    ])
    .with_property_schema(vec![
        PropertySchema::new("name", "Report name", ReflectedType::parse_str("String"))
            .with_default(PropertyValue::String("Performance Report".into())),
        PropertySchema::new(
            "risk_free_rate",
            "Risk-free rate (annual)",
            ReflectedType::parse_str("f64"),
        )
        .with_default(PropertyValue::Float(0.0)),
        PropertySchema::new(
            "rolling_window",
            "Rolling window (bars)",
            ReflectedType::parse_str("u32"),
        )
        .with_default(PropertyValue::Int(63)),
        PropertySchema::new(
            "benchmark_mode",
            "Benchmark mode",
            ReflectedType::parse_str("String"),
        )
        .with_default(PropertyValue::String("auto".into()))
        .with_tooltip("auto | wired | symbol"),
        PropertySchema::new(
            "benchmark_symbol",
            "Benchmark symbol",
            ReflectedType::parse_str("String"),
        )
        .with_default(PropertyValue::String("SPY".into())),
    ])
    .with_doc("Performance Analytics")
    .with_version(1)
}

/// Build the full finance node catalog keyed by Graphy `node_type` id.
pub fn finance_node_catalog() -> HashMap<String, NodeMetadata> {
    let mut catalog = HashMap::new();

    let entries: Vec<NodeMetadata> = vec![
        financial_asset_metadata(),
        financial_return_asset_metadata(),
        otl_operator_metadata(),
        ta_uber_metadata(type_id::TA_TREND, TaArchetype::Trend),
        ta_uber_metadata(type_id::TA_VOLATILITY, TaArchetype::Volatility),
        ta_uber_metadata(type_id::TA_OSCILLATOR, TaArchetype::Oscillator),
        ta_uber_metadata(type_id::TA_CHANNEL, TaArchetype::Channel),
        portfolio_integrator_metadata(),
        performance_analytics_metadata(),
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
        assert!(catalog.contains_key(type_id::FINANCIAL_RETURN_ASSET));
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
