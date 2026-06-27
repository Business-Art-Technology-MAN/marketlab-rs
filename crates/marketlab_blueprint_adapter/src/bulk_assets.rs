//! Bulk finance asset ingestion — symbols, bundled tickers, and CSV files.

use std::collections::HashSet;
use std::path::Path;

use crate::blueprint::finance_property_defaults;
use crate::asset_data::normalize_finance_file_path;
use crate::taxonomy_index::{finance_asset_properties_for_symbol, FinanceDatabaseIndex};
use crate::types::type_id;

/// Detected CSV layout for a finance price-source file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FinanceCsvKind {
    Ohlc,
    ReturnSeries,
}

/// One asset node to place on the finance canvas.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FinanceBulkAssetDraft {
    pub node_type: String,
    pub symbol: String,
    pub properties: std::collections::HashMap<String, String>,
}

/// Bundled OHLC sample tickers shipped with the repo (`pulsar_marketlab/data/*.csv`).
pub fn list_bundled_finance_symbols() -> Vec<String> {
    let data_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../pulsar_marketlab/data");
    let mut symbols = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&data_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("csv") {
                continue;
            }
            if let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) {
                if !stem.is_empty() {
                    symbols.push(stem.to_ascii_uppercase());
                }
            }
        }
    }
    symbols.sort();
    symbols.dedup();
    symbols
}

/// Parse comma / semicolon / whitespace separated ticker tokens.
pub fn parse_symbol_tokens(text: &str) -> Vec<String> {
    text.split(|ch: char| ch == ',' || ch == ';' || ch.is_whitespace())
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(|token| token.to_ascii_uppercase())
        .collect()
}

/// Classify a CSV file from its header row.
pub fn infer_csv_kind(path: &Path) -> Result<FinanceCsvKind, String> {
    if !path.is_file() {
        return Err(format!("file not found: {}", path.display()));
    }
    let mut reader = csv::Reader::from_path(path)
        .map_err(|error| format!("open {}: {error}", path.display()))?;
    let headers = reader
        .headers()
        .map_err(|error| format!("read header: {error}"))?
        .clone();

    let has_open = header_has(&headers, "Open");
    let has_close = header_has(&headers, "Close") || header_has(&headers, "Adj Close");
    if has_open && has_close {
        return Ok(FinanceCsvKind::Ohlc);
    }

    if header_has(&headers, "Date") && headers.len() >= 2 {
        return Ok(FinanceCsvKind::ReturnSeries);
    }

    Err(format!(
        "unrecognized CSV headers in `{}` (expected OHLC or Date,<Return>)",
        path.display()
    ))
}

/// Build a draft for a ticker symbol (bundled or custom OHLC asset).
pub fn finance_bulk_draft_from_symbol(
    symbol: &str,
    database: Option<&FinanceDatabaseIndex>,
) -> FinanceBulkAssetDraft {
    let mut properties = finance_asset_properties_for_symbol(symbol, database);
    properties.entry("csv_path".to_string()).or_default();
    let symbol = properties
        .get("symbol")
        .cloned()
        .unwrap_or_else(|| symbol.trim().to_ascii_uppercase());
    FinanceBulkAssetDraft {
        node_type: type_id::FINANCIAL_ASSET.to_string(),
        symbol,
        properties,
    }
}

/// Build a draft from a CSV path (OHLC or return-series).
pub fn finance_bulk_draft_from_csv_path(
    path: &Path,
    database: Option<&FinanceDatabaseIndex>,
) -> Result<FinanceBulkAssetDraft, String> {
    let kind = infer_csv_kind(path)?;
    let normalized = normalize_finance_file_path(&path.to_string_lossy());

    match kind {
        FinanceCsvKind::Ohlc => {
            let symbol = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| value.to_ascii_uppercase())
                .unwrap_or_else(|| "ASSET".to_string());
            let mut properties = finance_asset_properties_for_symbol(&symbol, database);
            properties.insert("csv_path".to_string(), normalized);
            let symbol = properties
                .get("symbol")
                .cloned()
                .unwrap_or(symbol);
            Ok(FinanceBulkAssetDraft {
                node_type: type_id::FINANCIAL_ASSET.to_string(),
                symbol,
                properties,
            })
        }
        FinanceCsvKind::ReturnSeries => {
            let header_symbol = read_return_series_header_symbol(path)?;
            let mut properties = finance_property_defaults(type_id::FINANCIAL_RETURN_ASSET);
            properties.insert("symbol".to_string(), header_symbol.clone());
            properties.insert("csv_path".to_string(), normalized);
            if properties
                .get("prim_path")
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
                .is_none()
            {
                let leaf = header_symbol.replace([' ', '/'], "_");
                properties.insert("prim_path".to_string(), format!("/MarketLab/Universe/{leaf}"));
            }
            Ok(FinanceBulkAssetDraft {
                node_type: type_id::FINANCIAL_RETURN_ASSET.to_string(),
                symbol: header_symbol,
                properties,
            })
        }
    }
}

/// Merge symbol picks and CSV paths into unique node drafts (symbols win over duplicate tickers).
pub fn collect_finance_bulk_drafts(
    symbols: &[String],
    csv_paths: &[String],
    database: Option<&FinanceDatabaseIndex>,
) -> Result<Vec<FinanceBulkAssetDraft>, String> {
    let mut drafts = Vec::new();
    let mut seen_ohlc_symbols = HashSet::new();
    let mut seen_csv_paths = HashSet::new();

    for symbol in symbols {
        let upper = symbol.trim().to_ascii_uppercase();
        if upper.is_empty() || !seen_ohlc_symbols.insert(upper.clone()) {
            continue;
        }
        drafts.push(finance_bulk_draft_from_symbol(&upper, database));
    }

    for path_str in csv_paths {
        let normalized = normalize_finance_file_path(path_str);
        if normalized.is_empty() || !seen_csv_paths.insert(normalized.clone()) {
            continue;
        }
        let path = Path::new(&normalized);
        let draft = finance_bulk_draft_from_csv_path(path, database)?;
        if draft.node_type == type_id::FINANCIAL_ASSET {
            let upper = draft.symbol.to_ascii_uppercase();
            if !seen_ohlc_symbols.insert(upper) {
                continue;
            }
        }
        drafts.push(draft);
    }

    Ok(drafts)
}

fn header_has(headers: &csv::StringRecord, name: &str) -> bool {
    headers
        .iter()
        .any(|header| header.trim().eq_ignore_ascii_case(name))
}

fn read_return_series_header_symbol(path: &Path) -> Result<String, String> {
    let mut reader = csv::Reader::from_path(path)
        .map_err(|error| format!("open {}: {error}", path.display()))?;
    let headers = reader
        .headers()
        .map_err(|error| format!("read header: {error}"))?
        .clone();
    let date_idx = headers
        .iter()
        .position(|header| header.trim().eq_ignore_ascii_case("Date"))
        .ok_or_else(|| format!("missing Date column in `{}`", path.display()))?;
    let value_idx = (0..headers.len())
        .find(|index| *index != date_idx)
        .ok_or_else(|| format!("missing return column in `{}`", path.display()))?;
    Ok(headers
        .get(value_idx)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("RETURN")
        .to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_bundled_sample_symbols() {
        let symbols = list_bundled_finance_symbols();
        assert!(symbols.contains(&"SPY".to_string()));
    }

    #[test]
    fn parses_symbol_tokens() {
        assert_eq!(
            parse_symbol_tokens("SPY, qqq\nIWM"),
            vec!["SPY".to_string(), "QQQ".to_string(), "IWM".to_string()]
        );
    }

    #[test]
    fn infers_return_series_csv_kind() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("winton.csv");
        std::fs::write(&path, "Date,Winton\n12/1/2024,0.01\n").expect("write");
        assert_eq!(infer_csv_kind(&path).expect("kind"), FinanceCsvKind::ReturnSeries);
        let draft = finance_bulk_draft_from_csv_path(&path, None).expect("draft");
        assert_eq!(draft.node_type, type_id::FINANCIAL_RETURN_ASSET);
        assert_eq!(draft.symbol, "Winton");
    }

    #[test]
    fn dedupes_symbol_and_csv_for_same_ticker() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("SPY.csv");
        std::fs::write(
            &path,
            "Date,Open,High,Low,Close\n2024-01-01,1,2,0.5,1.5\n",
        )
        .expect("write");
        let path_str = path.to_string_lossy().to_string();
        let drafts =
            collect_finance_bulk_drafts(&["SPY".to_string()], &[path_str], None).expect("drafts");
        assert_eq!(drafts.len(), 1);
        assert_eq!(drafts[0].symbol, "SPY");
    }
}
