//! Compile-time taxonomy resolution for [`metadata_library.usda`] class chains.
//!
//! OpenUSD 0.3 in Rust does not run full native composition; asset prims receive
//! flattened metadata opinions at USDA compose time via [`flatten_asset_metadata`].

/// Flattened asset metadata applied to operational `FinancialAsset` prims.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct FlattenedAssetMetadata {
    pub asset_class: String,
    pub provider: String,
    pub category: String,
    pub sub_category: String,
    pub exchange_mic: String,
    pub exchange_region: String,
    pub trading_currency: String,
}

/// Symbol-specific taxonomy overrides (extend as the universe grows).
fn symbol_taxonomy_chain(symbol: &str) -> (&'static [&'static str], &'static str) {
    let upper = symbol.to_ascii_uppercase();
    match upper.as_str() {
        "AAPL" | "MSFT" | "GOOG" | "GOOGL" => (
            &["MlabEquityBase", "Sector_Technology", "Industry_Software"],
            "Exchange_NASDAQ",
        ),
        "NVDA" | "AMD" | "INTC" => (
            &["MlabEquityBase", "Sector_Technology", "Industry_Semiconductors"],
            "Exchange_NASDAQ",
        ),
        "JPM" | "BAC" | "WFC" => (
            &["MlabEquityBase", "Sector_Financials", "Industry_Banks"],
            "Exchange_NYQ",
        ),
        "GS" | "MS" => (
            &["MlabEquityBase", "Sector_Financials", "Industry_Capital_Markets"],
            "Exchange_NYQ",
        ),
        "SPY" | "QQQ" | "IVV" => (
            &["MlabEtfBase", "Etf_Equity_LargeCap"],
            "Exchange_NYQ",
        ),
        "TLT" | "IEF" | "SHY" => (
            &["MlabEtfBase", "Etf_FixedIncome_Treasury"],
            "Exchange_NYQ",
        ),
        "SAP" => (
            &["MlabEquityBase", "Sector_Technology", "Industry_Software"],
            "Exchange_FRA",
        ),
        _ => (&["MlabEquityBase"], "Exchange_NYQ"),
    }
}

/// Resolve a multi-inheritance taxonomy chain into flattened token/string opinions.
pub fn flatten_asset_metadata(symbol: &str, declared_asset_class: Option<&str>) -> FlattenedAssetMetadata {
    let (classes, exchange_class) = symbol_taxonomy_chain(symbol);
    let mut meta = FlattenedAssetMetadata::default();
    for class in classes {
        apply_class_defaults(class, &mut meta);
    }
    apply_class_defaults(exchange_class, &mut meta);
    if let Some(asset_class) = declared_asset_class.filter(|value| !value.is_empty()) {
        meta.asset_class = asset_class.to_string();
    }
    if meta.asset_class.is_empty() {
        meta.asset_class = "Equity".to_string();
    }
    if meta.provider.is_empty() {
        meta.provider = "yahoo".to_string();
    }
    meta
}

fn apply_class_defaults(class_name: &str, meta: &mut FlattenedAssetMetadata) {
    match class_name {
        "MlabEquityBase" => {
            meta.asset_class = "Equity".to_string();
            meta.provider = "yahoo".to_string();
        }
        "MlabEtfBase" => {
            meta.asset_class = "ETF".to_string();
            meta.provider = "yahoo".to_string();
        }
        "Sector_Technology" => meta.category = "Information Technology".to_string(),
        "Industry_Software" => meta.sub_category = "Software Application & Systems".to_string(),
        "Industry_Semiconductors" => meta.sub_category = "Semiconductors & Equipment".to_string(),
        "Sector_Financials" => meta.category = "Financials".to_string(),
        "Industry_Banks" => meta.sub_category = "Diversified Banks".to_string(),
        "Industry_Capital_Markets" => {
            meta.sub_category = "Capital Markets & Asset Management".to_string();
        }
        "Etf_Equity_LargeCap" => {
            meta.category = "Equity".to_string();
            meta.sub_category = "Large Cap Blend".to_string();
        }
        "Etf_FixedIncome_Treasury" => {
            meta.category = "Fixed Income".to_string();
            meta.sub_category = "Government US Treasury".to_string();
        }
        "Exchange_NASDAQ" => {
            meta.exchange_mic = "XNAS".to_string();
            meta.exchange_region = "US".to_string();
            meta.trading_currency = "USD".to_string();
        }
        "Exchange_NYQ" => {
            meta.exchange_mic = "XNYS".to_string();
            meta.exchange_region = "US".to_string();
            meta.trading_currency = "USD".to_string();
        }
        "Exchange_FRA" => {
            meta.exchange_mic = "XFRA".to_string();
            meta.exchange_region = "Germany".to_string();
            meta.trading_currency = "EUR".to_string();
        }
        _ => {}
    }
}

pub const METADATA_LIBRARY_USDA: &str = include_str!("../resources/usd/metadata_library.usda");
pub const METADATA_LIBRARY_SIDECAR_FILENAME: &str = "metadata_library.usda";
pub const METADATA_SUBLAYER_REF: &str = "@./metadata_library.usda@";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aapl_inherits_software_and_nasdaq() {
        let meta = flatten_asset_metadata("AAPL", None);
        assert_eq!(meta.asset_class, "Equity");
        assert_eq!(meta.category, "Information Technology");
        assert_eq!(meta.sub_category, "Software Application & Systems");
        assert_eq!(meta.exchange_mic, "XNAS");
    }

    #[test]
    fn spy_inherits_etf_large_cap() {
        let meta = flatten_asset_metadata("SPY", None);
        assert_eq!(meta.asset_class, "ETF");
        assert_eq!(meta.sub_category, "Large Cap Blend");
    }
}
