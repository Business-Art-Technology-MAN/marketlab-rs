//! In-memory taxonomy and FinanceDatabase indexes (cold-path build, hot-path lookup).

use std::collections::HashMap;
use std::path::Path;
use std::sync::OnceLock;

use openusd::sdf::schema::FieldKey;
use openusd::Stage;
use pulsar_marketlab_core::{
    exchange_token_to_mic, flatten_asset_metadata, map_sector_to_inputs, EquityCatalogRow,
    FINANCE_DATABASE_EQUITIES_LAYER_FILENAME,
};

use crate::usd_persistence::UsdTransaction;

static TAXONOMY_INDEX: OnceLock<TaxonomyIndex> = OnceLock::new();

/// Built once at startup from embedded metadata_library opinions (via flatten_asset_metadata).
#[derive(Clone, Debug, Default)]
pub struct TaxonomyIndex;

impl TaxonomyIndex {
    pub fn global() -> &'static Self {
        TAXONOMY_INDEX.get_or_init(Self::default)
    }
}

/// Optional symbol catalog loaded from `finance_database_equities.usda` beside a project.
#[derive(Clone, Debug, Default)]
pub struct FinanceDatabaseIndex {
    rows: HashMap<String, EquityCatalogRow>,
}

impl FinanceDatabaseIndex {
    pub fn empty() -> Self {
        Self::default()
    }

    /// Parse equities sidecar once (cold path). Returns empty index when the file is missing.
    pub fn load_from_project_dir(project_dir: impl AsRef<Path>) -> Self {
        let path = project_dir
            .as_ref()
            .join(FINANCE_DATABASE_EQUITIES_LAYER_FILENAME);
        if !path.is_file() {
            return Self::empty();
        }
        Self::load_from_path(&path).unwrap_or_default()
    }

    pub fn load_from_path(path: impl AsRef<Path>) -> Result<Self, String> {
        let tx = UsdTransaction::open(path.as_ref()).map_err(|e| e.to_string())?;
        let stage = tx.stage();
        let mut rows = HashMap::new();
        walk_equity_prims(stage, "/MarketLab", &mut rows);
        Ok(Self { rows })
    }

    pub fn get(&self, symbol: &str) -> Option<&EquityCatalogRow> {
        self.rows.get(&symbol.to_ascii_uppercase())
    }
}

fn walk_equity_prims(stage: &Stage, path: &str, out: &mut HashMap<String, EquityCatalogRow>) {
    let Ok(children) = stage.prim_children(path) else {
        return;
    };
    for child in children {
        let child_path = format!("{path}/{child}");
        if let Some(symbol) = read_symbol(stage, &child_path) {
            let sector = field_string(stage, &child_path, "info:sector").unwrap_or_default();
            let industry_group =
                field_string(stage, &child_path, "info:industry_group").unwrap_or_default();
            let industry = field_string(stage, &child_path, "info:industry").unwrap_or_default();
            let exchange = field_string(stage, &child_path, "info:exchange").unwrap_or_default();
            out.insert(
                symbol.to_ascii_uppercase(),
                EquityCatalogRow {
                    symbol: symbol.to_ascii_uppercase(),
                    currency: field_string(stage, &child_path, "info:currency").unwrap_or_default(),
                    sector,
                    industry_group,
                    industry,
                    exchange,
                    country: field_string(stage, &child_path, "info:country").unwrap_or_default(),
                    state: field_string(stage, &child_path, "info:state").unwrap_or_default(),
                    zipcode: field_string(stage, &child_path, "info:zipcode").unwrap_or_default(),
                    market_cap_class: field_string(stage, &child_path, "info:market_cap_class")
                        .unwrap_or_default(),
                },
            );
        }
        walk_equity_prims(stage, &child_path, out);
    }
}

fn read_symbol(stage: &Stage, prim_path: &str) -> Option<String> {
    field_string(stage, prim_path, "inputs:symbol")
        .or_else(|| prim_path.rsplit('/').next().map(str::to_string))
        .filter(|s| !s.is_empty())
}

fn field_string(stage: &Stage, prim_path: &str, attribute: &str) -> Option<String> {
    let property_path = format!("{prim_path}.{attribute}");
    stage
        .field::<String>(property_path.as_str(), FieldKey::Default)
        .ok()
        .flatten()
        .or_else(|| stage.field::<String>(property_path.as_str(), "default").ok().flatten())
        .map(|token| token.trim_matches('"').to_string())
        .filter(|value| !value.is_empty())
}

/// Merge taxonomy + optional database row into finance node property strings.
pub fn finance_asset_properties_for_symbol(
    symbol: &str,
    database: Option<&FinanceDatabaseIndex>,
) -> HashMap<String, String> {
    let upper = symbol.trim().to_ascii_uppercase();
    if upper.is_empty() {
        return HashMap::new();
    }

    let meta = flatten_asset_metadata(&upper, None);
    let mut props = HashMap::from([
        ("symbol".to_string(), upper.clone()),
        ("asset_class".to_string(), meta.asset_class),
        ("category".to_string(), meta.category),
        ("sub_category".to_string(), meta.sub_category),
        ("exchange_mic".to_string(), meta.exchange_mic),
        (
            "prim_path".to_string(),
            format!("/MarketLab/Universe/{upper}"),
        ),
    ]);

    if let Some(row) = database.and_then(|db| db.get(&upper)) {
        let (category, sub_category) = map_sector_to_inputs(&row.sector, &row.industry_group);
        if !category.is_empty() {
            props.insert("category".to_string(), category);
        }
        if !sub_category.is_empty() {
            props.insert("sub_category".to_string(), sub_category);
        }
        let mic = exchange_token_to_mic(&row.exchange);
        if !mic.is_empty() {
            props.insert("exchange_mic".to_string(), mic);
        }
        insert_info_field(&mut props, "info_sector", &row.sector);
        insert_info_field(&mut props, "info_industry_group", &row.industry_group);
        insert_info_field(&mut props, "info_industry", &row.industry);
        insert_info_field(&mut props, "info_currency", &row.currency);
        insert_info_field(&mut props, "info_country", &row.country);
        insert_info_field(&mut props, "info_state", &row.state);
        insert_info_field(&mut props, "info_zipcode", &row.zipcode);
        insert_info_field(&mut props, "info_market_cap_class", &row.market_cap_class);
        if !row.exchange.is_empty() {
            props.insert("info_exchange".to_string(), row.exchange.clone());
        }
    }

    if !meta.provider.is_empty() {
        props.insert("provider".to_string(), meta.provider);
    }

    props
}

fn insert_info_field(props: &mut HashMap<String, String>, key: &str, value: &str) {
    if !value.trim().is_empty() {
        props.insert(key.to_string(), value.trim().to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aapl_taxonomy_autofill_has_sector_fields() {
        let props = finance_asset_properties_for_symbol("AAPL", None);
        assert_eq!(props.get("asset_class").map(String::as_str), Some("Equity"));
        assert_eq!(
            props.get("category").map(String::as_str),
            Some("Information Technology")
        );
        assert_eq!(props.get("prim_path").map(String::as_str), Some("/MarketLab/Universe/AAPL"));
    }
}
