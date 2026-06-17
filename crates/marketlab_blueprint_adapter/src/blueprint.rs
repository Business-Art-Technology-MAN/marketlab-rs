//! Helpers for registering MarketLab finance nodes in Plugin_Blueprints.

use std::collections::HashMap;

use graphy::{NodeMetadata, PropertyValue};

use crate::metadata::finance_node_catalog;
use crate::types::type_id;

/// Canonical data-pin type for all finance signal streams (price, signal, wealth).
pub const FINANCE_SIGNAL_TYPE: &str = "MarketLabSignalSeries";

/// Human-readable palette label for a finance node type id.
pub fn finance_display_label(node_type: &str) -> Option<&'static str> {
    match node_type {
        type_id::FINANCIAL_ASSET => Some("Financial Asset"),
        type_id::OTL_OPERATOR => Some("OTL Operator"),
        type_id::TA_TREND => Some("TA Trend"),
        type_id::TA_VOLATILITY => Some("TA Volatility"),
        type_id::TA_OSCILLATOR => Some("TA Oscillator"),
        type_id::TA_CHANNEL => Some("TA Channel"),
        type_id::PORTFOLIO_INTEGRATOR => Some("Portfolio Integrator"),
        _ => None,
    }
}

/// Primary data output pin id for a finance node (Blueprint editor pin names).
pub fn finance_primary_output_pin(node_type: &str) -> Option<&'static str> {
    match node_type {
        type_id::FINANCIAL_ASSET => Some("close"),
        type_id::OTL_OPERATOR | type_id::TA_TREND | type_id::TA_VOLATILITY
        | type_id::TA_OSCILLATOR | type_id::TA_CHANNEL => Some("result"),
        type_id::PORTFOLIO_INTEGRATOR => Some("wealth"),
        _ => None,
    }
}

/// Palette icon per finance category.
pub fn finance_category_icon(category: &str) -> &'static str {
    match category {
        crate::types::category::UNIVERSE => "📈",
        crate::types::category::ANALYTICS => "ƒ",
        crate::types::category::PORTFOLIOS => "⚖",
        _ => "◆",
    }
}

/// Merge finance [`NodeMetadata`] entries into a Graphy metadata map (PBGC + MarketLab).
pub fn merge_finance_node_metadata(metadata: &mut HashMap<String, NodeMetadata>) {
    for (id, meta) in finance_node_catalog() {
        metadata.insert(id, meta);
    }
}

/// Returns true when `node_type` is a MarketLab finance graph node.
pub fn is_marketlab_finance_node(node_type: &str) -> bool {
    node_type.starts_with("marketlab.")
}

/// Whether two pin type names may connect in the finance graph.
pub fn finance_data_types_compatible(a: &str, b: &str) -> bool {
    fn is_finance_stream(type_name: &str) -> bool {
        type_name.starts_with("MarketLab")
    }
    is_finance_stream(a) && is_finance_stream(b)
}

/// Inspector field descriptor for a finance node property.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FinancePropertyField {
    pub id: String,
    pub label: String,
}

/// Property schema rows for the Details panel (finance nodes only).
pub fn finance_property_fields(node_type: &str) -> Vec<FinancePropertyField> {
    let catalog = finance_node_catalog();
    let Some(meta) = catalog.get(node_type) else {
        return Vec::new();
    };
    meta.property_schema
        .iter()
        .map(|schema| FinancePropertyField {
            id: schema.id.clone(),
            label: schema.label.clone(),
        })
        .collect()
}

/// Default property values when a finance node is placed on the canvas.
pub fn finance_property_defaults(node_type: &str) -> HashMap<String, String> {
    let catalog = finance_node_catalog();
    let Some(meta) = catalog.get(node_type) else {
        return HashMap::new();
    };

    let mut properties = HashMap::new();
    for schema in &meta.property_schema {
        let value = schema
            .default_value
            .as_ref()
            .map(property_value_to_string)
            .unwrap_or_else(|| empty_default_for_type(&schema.ty.to_type_string()));
        properties.insert(schema.id.clone(), value);
    }
    properties
}

fn property_value_to_string(value: &PropertyValue) -> String {
    match value {
        PropertyValue::String(text) => text.clone(),
        PropertyValue::Int(number) => number.to_string(),
        PropertyValue::Float(number) => number.to_string(),
        PropertyValue::Bool(flag) => flag.to_string(),
        PropertyValue::Enum { variant, .. } => variant.clone(),
        PropertyValue::AssetRef { path, .. } => path.clone(),
        PropertyValue::Null => String::new(),
        PropertyValue::Array(values) => values
            .iter()
            .map(property_value_to_string)
            .collect::<Vec<_>>()
            .join(", "),
        PropertyValue::Struct { fields, .. } => fields
            .iter()
            .map(|(key, value)| format!("{key}={}", property_value_to_string(value)))
            .collect::<Vec<_>>()
            .join(", "),
    }
}

fn empty_default_for_type(type_name: &str) -> String {
    match type_name {
        "u32" | "i32" | "u64" | "i64" | "usize" | "isize" => "0".to_string(),
        "f32" | "f64" => "0".to_string(),
        "bool" => "false".to_string(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finance_signal_types_are_mutually_compatible() {
        assert!(finance_data_types_compatible(
            FINANCE_SIGNAL_TYPE,
            "MarketLabPriceSeries"
        ));
    }

    #[test]
    fn finance_defaults_include_csv_path() {
        let defaults = finance_property_defaults(type_id::FINANCIAL_ASSET);
        assert!(defaults.contains_key("csv_path"));
        assert!(defaults.contains_key("symbol"));
    }
}
