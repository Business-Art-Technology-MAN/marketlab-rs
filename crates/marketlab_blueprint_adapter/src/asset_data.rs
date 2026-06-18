//! CSV resolution and OHLC loading for finance asset nodes (shared by sweep + UI preview).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use pulsar_marketlab_core::StageGraphSnapshot;

const DEFAULT_BAR_COUNT: usize = 252;
const FLAT_FALLBACK_PRICE: f64 = 100.0;

/// One OHLC bar for chart preview.
#[derive(Clone, Debug, PartialEq)]
pub struct FinanceOhlcBar {
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
}

/// Loaded asset price history for the finance editor OHLC panel.
#[derive(Clone, Debug, PartialEq)]
pub struct FinanceAssetPreview {
    pub symbol: String,
    pub source_path: Option<PathBuf>,
    pub loaded_from_csv: bool,
    pub synthetic: bool,
    pub warnings: Vec<String>,
    pub bars: Vec<FinanceOhlcBar>,
}

impl FinanceAssetPreview {
    pub fn close_series(&self) -> Vec<f64> {
        self.bars.iter().map(|bar| bar.close).collect()
    }
}

/// Load OHLC bars for a financial asset using the same CSV resolver as sweep.
pub fn load_finance_asset_preview(
    symbol: &str,
    explicit_csv_path: Option<&str>,
) -> FinanceAssetPreview {
    let symbol = symbol.trim().to_ascii_uppercase();
    let mut warnings = Vec::new();
    let mut candidates = resolve_asset_csv_candidates(&symbol, explicit_csv_path);
    candidates.sort_by_key(|path| path.is_bundled);
    candidates.dedup_by(|left, right| left.path == right.path);

    for candidate in candidates {
        match load_ohlc_bars(&candidate.path) {
            Ok(bars) if !bars.is_empty() => {
                return FinanceAssetPreview {
                    symbol,
                    source_path: Some(candidate.path),
                    loaded_from_csv: true,
                    synthetic: false,
                    warnings,
                    bars,
                };
            }
            Ok(_) => warnings.push(format!(
                "CSV `{}` has too few OHLC rows",
                candidate.path.display()
            )),
            Err(error) => warnings.push(format!(
                "CSV load failed for `{}` ({error})",
                candidate.path.display()
            )),
        }
    }

    warnings.push(format!(
        "{symbol}: using flat synthetic OHLC — set csv_path or leave empty for bundled data/{symbol}.csv"
    ));
    FinanceAssetPreview {
        symbol,
        source_path: None,
        loaded_from_csv: false,
        synthetic: true,
        warnings,
        bars: flat_ohlc_series(DEFAULT_BAR_COUNT),
    }
}

/// OHLC previews keyed by graph node id (post-compile cache for the finance editor).
pub fn finance_asset_previews_for_snapshot(
    snapshot: &StageGraphSnapshot,
    node_prim_paths: &HashMap<String, String>,
) -> HashMap<String, FinanceAssetPreview> {
    let mut previews = HashMap::new();
    for (node_id, prim_path) in node_prim_paths {
        let Some(prim) = snapshot.prims.iter().find(|prim| &prim.path == prim_path) else {
            continue;
        };
        if prim.type_name != "FinancialAsset" {
            continue;
        }
        let symbol = prim
            .attributes
            .get("inputs:symbol")
            .map(|value| value.as_str())
            .unwrap_or("SPY");
        let csv_path = prim.attributes.get("inputs:csv_path").map(|value| value.as_str());
        previews.insert(
            node_id.clone(),
            load_finance_asset_preview(symbol, csv_path),
        );
    }
    previews
}

/// Close prices for engine sweep (same resolver as [`load_finance_asset_preview`]).
pub(crate) fn load_asset_close_series(
    symbol: &str,
    explicit_csv: Option<&String>,
    prim_path: &str,
    warnings: &mut Vec<String>,
) -> (Vec<f64>, bool, bool) {
    let preview = load_finance_asset_preview(
        symbol,
        explicit_csv.map(|value| value.as_str()),
    );
    warnings.extend(
        preview
            .warnings
            .iter()
            .map(|message| format!("{prim_path}: {message}")),
    );
    let closes = preview.close_series();
    if preview.loaded_from_csv {
        (closes, true, false)
    } else {
        (closes, false, true)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CsvCandidate {
    path: PathBuf,
    is_bundled: bool,
}

fn resolve_asset_csv_candidates(symbol: &str, explicit: Option<&str>) -> Vec<CsvCandidate> {
    let mut candidates = Vec::new();

    if let Some(path) = explicit
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
    {
        if !is_bare_ticker_token(path) {
            if let Some(resolved) = resolve_existing_path(path) {
                candidates.push(CsvCandidate {
                    path: resolved,
                    is_bundled: false,
                });
            }
        } else if let Some(bundled) = bundled_data_csv(path) {
            candidates.push(CsvCandidate {
                path: bundled,
                is_bundled: true,
            });
        }
    }

    if !symbol.is_empty() {
        if let Some(bundled) = bundled_data_csv(symbol) {
            candidates.push(CsvCandidate {
                path: bundled,
                is_bundled: true,
            });
        }
        for path in bundled_csv_candidates(symbol) {
            candidates.push(CsvCandidate {
                path,
                is_bundled: true,
            });
        }
    }

    candidates
}

fn is_bare_ticker_token(value: &str) -> bool {
    let token = value.trim();
    !token.contains(['/', '\\'])
        && !token.contains('.')
        && !token.is_empty()
        && token.len() <= 6
        && token.chars().all(|ch| ch.is_ascii_alphanumeric())
}

fn bundled_data_csv(symbol: &str) -> Option<PathBuf> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../pulsar_marketlab/data")
        .join(format!("{}.csv", symbol.to_ascii_uppercase()));
    path.is_file().then_some(path)
}

fn resolve_existing_path(path: &str) -> Option<PathBuf> {
    let resolved = resolve_csv_path(path);
    resolved.is_file().then_some(resolved)
}

fn resolve_csv_path(path: &str) -> PathBuf {
    let candidate = Path::new(path);
    if candidate.is_file() {
        return candidate.to_path_buf();
    }
    if let Ok(cwd) = std::env::current_dir() {
        let joined = cwd.join(candidate);
        if joined.is_file() {
            return joined;
        }
    }
    candidate.to_path_buf()
}

fn bundled_csv_candidates(symbol: &str) -> Vec<PathBuf> {
    let rel = format!("crates/pulsar_marketlab/data/{symbol}.csv");
    let mut candidates = vec![
        Path::new(&rel).to_path_buf(),
        Path::new("data").join(format!("{symbol}.csv")),
    ];
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join(&rel));
        let mut dir = cwd.as_path();
        while let Some(parent) = dir.parent() {
            candidates.push(parent.join(&rel));
            dir = parent;
        }
    }
    candidates
}

fn load_ohlc_bars(path: &Path) -> Result<Vec<FinanceOhlcBar>, String> {
    if !path.is_file() {
        return Err(format!("file not found at {}", path.display()));
    }

    let mut reader = csv::Reader::from_path(path)
        .map_err(|error| format!("open {}: {error}", path.display()))?;
    let headers = reader
        .headers()
        .map_err(|error| format!("read header: {error}"))?
        .clone();

    let open_idx = csv_header_index(&headers, &["Open"]).ok_or_else(|| missing_columns(path, &headers))?;
    let high_idx = csv_header_index(&headers, &["High"]).ok_or_else(|| missing_columns(path, &headers))?;
    let low_idx = csv_header_index(&headers, &["Low"]).ok_or_else(|| missing_columns(path, &headers))?;
    let close_idx =
        csv_header_index(&headers, &["Adj Close", "Close"]).ok_or_else(|| missing_columns(path, &headers))?;

    let mut bars = Vec::new();
    for (offset, record) in reader.records().enumerate() {
        let record = record.map_err(|error| format!("row {}: {error}", offset + 2))?;
        let first = record.get(0).unwrap_or("").trim();
        if first.eq_ignore_ascii_case("Ticker") || first.eq_ignore_ascii_case("Date") {
            continue;
        }
        let Some((open, high, low, close)) = parse_ohlc_row(&record, open_idx, high_idx, low_idx, close_idx) else {
            continue;
        };
        bars.push(FinanceOhlcBar {
            open,
            high,
            low,
            close,
        });
    }

    if bars.is_empty() {
        return Err(format!("no OHLC rows parsed from {}", path.display()));
    }
    Ok(bars)
}

fn missing_columns(path: &Path, headers: &csv::StringRecord) -> String {
    format!(
        "missing OHLC columns in `{}` (headers: {})",
        path.display(),
        headers.iter().collect::<Vec<_>>().join(", ")
    )
}

fn parse_ohlc_row(
    record: &csv::StringRecord,
    open_idx: usize,
    high_idx: usize,
    low_idx: usize,
    close_idx: usize,
) -> Option<(f64, f64, f64, f64)> {
    let parse = |index: usize| {
        let raw = record.get(index)?.trim().trim_matches('"');
        raw.parse::<f64>().ok()
    };
    let open = parse(open_idx)?;
    let high = parse(high_idx)?;
    let low = parse(low_idx)?;
    let close = parse(close_idx)?;
    Some((open, high, low, close))
}

fn csv_header_index(headers: &csv::StringRecord, names: &[&str]) -> Option<usize> {
    for name in names {
        if let Some(index) = headers.iter().position(|cell| cell.trim() == *name) {
            return Some(index);
        }
    }
    None
}

fn flat_ohlc_series(len: usize) -> Vec<FinanceOhlcBar> {
    vec![
        FinanceOhlcBar {
            open: FLAT_FALLBACK_PRICE,
            high: FLAT_FALLBACK_PRICE,
            low: FLAT_FALLBACK_PRICE,
            close: FLAT_FALLBACK_PRICE,
        };
        len.max(1)
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_bundled_spy_ohlc_without_explicit_path() {
        let preview = load_finance_asset_preview("SPY", None);
        assert!(preview.loaded_from_csv, "{:?}", preview.warnings);
        assert!(!preview.synthetic);
        assert!(preview.bars.len() >= 2);
        assert!(preview.bars[0].close > 0.0);
    }

    #[test]
    fn close_series_matches_bar_closes() {
        let preview = load_finance_asset_preview("SPY", None);
        let closes: Vec<f64> = preview.bars.iter().map(|bar| bar.close).collect();
        assert_eq!(preview.close_series(), closes);
    }

    #[test]
    fn finance_asset_previews_for_snapshot_reads_prim_attributes() {
        use pulsar_marketlab_core::{StageGraphPrim, StageGraphSnapshot};

        let mut paths = HashMap::new();
        paths.insert("node-1".to_string(), "/MarketLab/Universe/SPY".to_string());
        let mut snapshot = StageGraphSnapshot::default();
        snapshot.prims.push(StageGraphPrim {
            path: "/MarketLab/Universe/SPY".to_string(),
            type_name: "FinancialAsset".to_string(),
            attributes: HashMap::from([
                ("inputs:symbol".to_string(), "SPY".to_string()),
                ("inputs:active".to_string(), "true".to_string()),
            ]),
        });

        let previews = finance_asset_previews_for_snapshot(&snapshot, &paths);
        let preview = previews.get("node-1").expect("preview");
        assert!(preview.loaded_from_csv, "{:?}", preview.warnings);
        assert!(!preview.bars.is_empty());
    }

    #[test]
    fn skips_modern_yahoo_ticker_metadata_row() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("modern.csv");
        std::fs::write(
            &path,
            "Price,Close,High,Low,Open,Volume\n\
             Ticker,SPY,SPY,SPY,SPY,SPY\n\
             Date,,,,,\n\
             2024-01-02,472.65,473.67,470.05,472.16,123456000\n",
        )
        .expect("write csv");
        let preview = load_finance_asset_preview("TEST", path.to_str());
        assert!(preview.loaded_from_csv, "{:?}", preview.warnings);
        assert_eq!(preview.bars.len(), 1);
        assert!((preview.bars[0].close - 472.65).abs() < f64::EPSILON);
    }
}
