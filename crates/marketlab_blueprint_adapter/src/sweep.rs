//! Run a timeline sweep from a compiled [`StageGraphSnapshot`].

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use pulsar_marketlab_core::{
    MarketLabGraphEngine, SharedPriceColumn, StageGraphPrim, StageGraphSnapshot,
};

const DEFAULT_BAR_COUNT: usize = 252;
const FLAT_FALLBACK_PRICE: f64 = 100.0;

/// Per-portfolio sweep output for the blueprint editor UI.
#[derive(Clone, Debug, PartialEq)]
pub struct FinancePortfolioSweepSummary {
    pub prim_path: String,
    pub label: String,
    pub initial_capital: f64,
    pub final_wealth: f64,
    pub return_pct: f64,
    pub wealth_series: Vec<f64>,
}

/// Result of running the MarketLab engine on a finance snapshot.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FinanceSweepResult {
    pub timeline_len: usize,
    pub assets_loaded: usize,
    pub assets_synthetic: usize,
    pub portfolios: Vec<FinancePortfolioSweepSummary>,
    pub warnings: Vec<String>,
    pub error: Option<String>,
}

impl FinanceSweepResult {
    pub fn summary_lines(&self) -> Vec<String> {
        if let Some(error) = &self.error {
            return vec![format!("Sweep failed: {error}")];
        }
        let mut lines = vec![format!(
            "Timeline: {} bars · Assets loaded: {} · Synthetic: {}",
            self.timeline_len, self.assets_loaded, self.assets_synthetic
        )];
        for portfolio in &self.portfolios {
            lines.push(format!(
                "{} — ${:.2} ({:+.2}%)",
                portfolio.label, portfolio.final_wealth, portfolio.return_pct
            ));
        }
        for warning in &self.warnings {
            lines.push(format!("Warning: {warning}"));
        }
        lines
    }

    pub fn succeeded(&self) -> bool {
        self.error.is_none() && !self.portfolios.is_empty()
    }
}

/// Compact unicode sparkline for wealth curves in the Details panel.
pub fn wealth_sparkline(values: &[f64], width: usize) -> String {
    const BARS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    if values.is_empty() {
        return "—".to_string();
    }
    let width = width.max(4);
    let step = (values.len() as f64 / width as f64).max(1.0);
    let sampled: Vec<f64> = (0..width)
        .map(|index| {
            let sample_index = ((index as f64) * step) as usize;
            values[sample_index.min(values.len() - 1)]
        })
        .collect();
    let min = sampled
        .iter()
        .copied()
        .fold(f64::INFINITY, f64::min);
    let max = sampled
        .iter()
        .copied()
        .fold(f64::NEG_INFINITY, f64::max);
    let range = (max - min).max(f64::EPSILON);
    sampled
        .iter()
        .map(|value| {
            let tier = (((value - min) / range) * 7.0).round() as usize;
            BARS[tier.min(7)]
        })
        .collect()
}

/// Execute one portfolio timeline sweep from a compiled stage snapshot.
pub fn run_finance_sweep(snapshot: &StageGraphSnapshot) -> FinanceSweepResult {
    let (vectors, load_meta) = build_asset_vectors(snapshot);
    if vectors.is_empty() {
        return FinanceSweepResult {
            error: Some(
                "No asset price data — set csv_path on Financial Asset nodes".to_string(),
            ),
            ..FinanceSweepResult::default()
        };
    }

    let timeline_len = vectors
        .values()
        .map(|column| column.as_slice().len())
        .max()
        .unwrap_or(DEFAULT_BAR_COUNT)
        .max(1);

    let mut warnings = load_meta.warnings;
    let engine = match MarketLabGraphEngine::compile_from_stage(snapshot) {
        Ok(engine) => engine,
        Err(error) => {
            return FinanceSweepResult {
                timeline_len,
                assets_loaded: load_meta.loaded,
                assets_synthetic: load_meta.synthetic,
                warnings,
                error: Some(error.to_string()),
                ..FinanceSweepResult::default()
            };
        }
    };

    let engine = match engine.compile_otl_scripts() {
        Ok(engine) => engine,
        Err(error) => {
            return FinanceSweepResult {
                timeline_len,
                assets_loaded: load_meta.loaded,
                assets_synthetic: load_meta.synthetic,
                warnings,
                error: Some(error.to_string()),
                ..FinanceSweepResult::default()
            };
        }
    };

    let mut engine = engine;
    let result = engine.execute_timeline(vectors, timeline_len);
    let mut portfolios = Vec::new();

    for prim in snapshot
        .prims
        .iter()
        .filter(|prim| prim.type_name == "PortfolioIntegrator")
    {
        let initial_capital = prim
            .attributes
            .get("inputs:initial_capital")
            .and_then(|value| value.parse::<f64>().ok())
            .unwrap_or(10_000_000.0);
        let label = portfolio_label(prim);
        let wealth_series = result
            .portfolio_results
            .get(&prim.path)
            .map(|integration| integration.wealth_series.clone())
            .or_else(|| {
                result
                    .streams
                    .iter()
                    .find(|stream| {
                        stream.prim_path == prim.path
                            && stream.attribute == "outputs:portfolio_wealth"
                    })
                    .map(|stream| stream.values.clone())
            })
            .unwrap_or_default();
        let final_wealth = wealth_series
            .last()
            .copied()
            .unwrap_or(initial_capital);
        let return_pct = if initial_capital.abs() > f64::EPSILON {
            ((final_wealth / initial_capital) - 1.0) * 100.0
        } else {
            0.0
        };
        portfolios.push(FinancePortfolioSweepSummary {
            prim_path: prim.path.clone(),
            label,
            initial_capital,
            final_wealth,
            return_pct,
            wealth_series,
        });
    }

    if portfolios.is_empty() {
        warnings.push("Sweep completed but no portfolio wealth streams were produced".to_string());
    }

    FinanceSweepResult {
        timeline_len,
        assets_loaded: load_meta.loaded,
        assets_synthetic: load_meta.synthetic,
        portfolios,
        warnings,
        error: None,
    }
}

struct AssetLoadMeta {
    loaded: usize,
    synthetic: usize,
    warnings: Vec<String>,
}

fn build_asset_vectors(snapshot: &StageGraphSnapshot) -> (HashMap<String, SharedPriceColumn>, AssetLoadMeta) {
    let mut vectors = HashMap::new();
    let mut meta = AssetLoadMeta {
        loaded: 0,
        synthetic: 0,
        warnings: Vec::new(),
    };

    for prim in snapshot
        .prims
        .iter()
        .filter(|prim| prim.type_name == "FinancialAsset")
    {
        let symbol = prim
            .attributes
            .get("inputs:symbol")
            .map(|value| value.trim().to_ascii_uppercase())
            .filter(|value| !value.is_empty())
            .or_else(|| {
                prim.path
                    .rsplit('/')
                    .next()
                    .map(|leaf| leaf.to_ascii_uppercase())
            })
            .unwrap_or_default();
        let explicit_csv = prim.attributes.get("inputs:csv_path");

        let (series, loaded, synthetic) =
            load_asset_close_series(&symbol, explicit_csv, &prim.path, &mut meta.warnings);
        if loaded {
            meta.loaded += 1;
        }
        if synthetic {
            meta.synthetic += 1;
        }

        vectors.insert(
            prim.path.clone(),
            SharedPriceColumn::from_series(Arc::from(series.into_boxed_slice())),
        );
    }

    (vectors, meta)
}

fn portfolio_label(prim: &StageGraphPrim) -> String {
    prim.attributes
        .get("inputs:id")
        .cloned()
        .filter(|value| !value.is_empty())
        .or_else(|| {
            prim.path
                .rsplit('/')
                .next()
                .map(str::to_string)
        })
        .unwrap_or_else(|| "Portfolio".to_string())
}

fn flat_series(len: usize) -> Vec<f64> {
    vec![FLAT_FALLBACK_PRICE; len.max(1)]
}

fn load_asset_close_series(
    symbol: &str,
    explicit_csv: Option<&String>,
    prim_path: &str,
    warnings: &mut Vec<String>,
) -> (Vec<f64>, bool, bool) {
    let mut candidates = resolve_asset_csv_candidates(symbol, explicit_csv);
    candidates.sort_by_key(|path| path.is_bundled);
    candidates.dedup_by(|left, right| left.path == right.path);

    for candidate in candidates {
        match load_close_prices(&candidate.path) {
            Ok(prices) if prices.len() >= 2 => return (prices, true, false),
            Ok(_) => warnings.push(format!(
                "{prim_path}: CSV `{}` has too few price rows",
                candidate.path.display()
            )),
            Err(error) => warnings.push(format!(
                "{prim_path}: CSV load failed for `{}` ({error})",
                candidate.path.display()
            )),
        }
    }

    warnings.push(format!(
        "{prim_path} ({symbol}): using flat synthetic prices — set csv_path to a Yahoo CSV or leave empty for bundled data/{}.csv",
        symbol.to_ascii_lowercase()
    ));
    (flat_series(DEFAULT_BAR_COUNT), false, true)
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CsvCandidate {
    path: std::path::PathBuf,
    is_bundled: bool,
}

fn resolve_asset_csv_candidates(symbol: &str, explicit: Option<&String>) -> Vec<CsvCandidate> {
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

fn bundled_data_csv(symbol: &str) -> Option<std::path::PathBuf> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../pulsar_marketlab/data")
        .join(format!("{}.csv", symbol.to_ascii_uppercase()));
    path.is_file().then_some(path)
}

fn resolve_existing_path(path: &str) -> Option<std::path::PathBuf> {
    let resolved = resolve_csv_path(path);
    resolved.is_file().then_some(resolved)
}

fn load_close_prices(path: &std::path::Path) -> Result<Vec<f64>, String> {
    if !path.is_file() {
        return Err(format!("file not found at {}", path.display()));
    }

    let mut reader = csv::Reader::from_path(path)
        .map_err(|error| format!("open {}: {error}", path.display()))?;
    let headers = reader
        .headers()
        .map_err(|error| format!("read header: {error}"))?
        .clone();

    let close_idx = csv_header_index(&headers, &["Adj Close", "Close"]).ok_or_else(|| {
        format!(
            "missing Close/Adj Close column in `{}` (headers: {})",
            path.display(),
            headers.iter().collect::<Vec<_>>().join(", ")
        )
    })?;

    let mut closes = Vec::new();
    for (offset, record) in reader.records().enumerate() {
        let record = record.map_err(|error| format!("row {}: {error}", offset + 2))?;
        let first = record.get(0).unwrap_or("").trim();
        if first.eq_ignore_ascii_case("Ticker") || first.eq_ignore_ascii_case("Date") {
            continue;
        }
        let Some(raw) = record.get(close_idx) else {
            continue;
        };
        let trimmed = raw.trim().trim_matches('"');
        if trimmed.is_empty() {
            continue;
        }
        let Ok(value) = trimmed.parse::<f64>() else {
            continue;
        };
        closes.push(value);
    }

    if closes.is_empty() {
        return Err(format!("no numeric close prices parsed from {}", path.display()));
    }
    Ok(closes)
}

fn csv_header_index(headers: &csv::StringRecord, names: &[&str]) -> Option<usize> {
    for name in names {
        if let Some(index) = headers.iter().position(|cell| cell.trim() == *name) {
            return Some(index);
        }
    }
    None
}

fn resolve_csv_path(path: &str) -> std::path::PathBuf {
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

fn bundled_csv_candidates(symbol: &str) -> Vec<std::path::PathBuf> {
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

#[cfg(test)]
mod tests {
    use pulsar_marketlab_core::{GraphCompileWire, StageGraphPrim};

    use super::*;
    use crate::snapshot::graph_description_to_stage_snapshot;
    use crate::types::type_id;
    use graphy::{Connection, ConnectionType, GraphDescription, NodeInstance, Position};

    #[test]
    fn sweep_runs_asset_to_portfolio_with_synthetic_prices() {
        let mut graph = GraphDescription::new("test");
        graph.add_node(NodeInstance::new(
            "spy",
            type_id::FINANCIAL_ASSET,
            Position::new(0.0, 0.0),
        ));
        graph.add_node(NodeInstance::new(
            "fund",
            type_id::PORTFOLIO_INTEGRATOR,
            Position::new(200.0, 0.0),
        ));
        graph.add_connection(Connection {
            source_node: "spy".into(),
            source_pin: "close".into(),
            target_node: "fund".into(),
            target_pin: "signal_0".into(),
            connection_type: ConnectionType::Data,
        });

        let snapshot = graph_description_to_stage_snapshot(&graph);
        let result = run_finance_sweep(&snapshot);
        assert!(result.error.is_none(), "{:?}", result.error);
        assert_eq!(result.portfolios.len(), 1);
        assert!(result.portfolios[0].final_wealth > 0.0);
    }

    #[test]
    fn sweep_handles_explicit_snapshot() {
        let snapshot = StageGraphSnapshot {
            prims: vec![
                StageGraphPrim {
                    path: "/MarketLab/Universe/SPY".into(),
                    type_name: "FinancialAsset".into(),
                    attributes: HashMap::from([("inputs:symbol".into(), "SPY".into())]),
                },
                StageGraphPrim {
                    path: "/MarketLab/Portfolios/fund".into(),
                    type_name: "PortfolioIntegrator".into(),
                    attributes: HashMap::from([
                        ("inputs:id".into(), "Allocation::EqualWeight".into()),
                        ("inputs:initial_capital".into(), "1000000".into()),
                    ]),
                },
            ],
            wires: vec![GraphCompileWire {
                source_prim_path: "/MarketLab/Universe/SPY".into(),
                target_prim_path: "/MarketLab/Portfolios/fund".into(),
                relationship: "inputs:sources".into(),
            }],
            ..StageGraphSnapshot::default()
        };
        let result = run_finance_sweep(&snapshot);
        assert!(result.succeeded());
    }

    #[test]
    fn sweep_loads_bundled_symbol_csv_without_explicit_path() {
        let snapshot = StageGraphSnapshot {
            prims: vec![StageGraphPrim {
                path: "/MarketLab/Universe/SPY".into(),
                type_name: "FinancialAsset".into(),
                attributes: HashMap::from([("inputs:symbol".into(), "SPY".into())]),
            }],
            ..StageGraphSnapshot::default()
        };
        let result = run_finance_sweep(&snapshot);
        assert!(result.assets_loaded >= 1, "{:?}", result);
        assert_eq!(result.assets_synthetic, 0);
    }

    #[test]
    fn sweep_runs_asset_through_ta_trend_to_portfolio() {
        let mut graph = GraphDescription::new("test");
        graph.add_node(NodeInstance::new(
            "spy",
            type_id::FINANCIAL_ASSET,
            Position::new(0.0, 0.0),
        ));
        graph.add_node(NodeInstance::new(
            "trend",
            type_id::TA_TREND,
            Position::new(100.0, 0.0),
        ));
        graph.add_node(NodeInstance::new(
            "fund",
            type_id::PORTFOLIO_INTEGRATOR,
            Position::new(200.0, 0.0),
        ));
        graph.add_connection(Connection {
            source_node: "spy".into(),
            source_pin: "close".into(),
            target_node: "trend".into(),
            target_pin: "source_stream".into(),
            connection_type: ConnectionType::Data,
        });
        graph.add_connection(Connection {
            source_node: "trend".into(),
            source_pin: "result".into(),
            target_node: "fund".into(),
            target_pin: "signal_0".into(),
            connection_type: ConnectionType::Data,
        });

        let snapshot = graph_description_to_stage_snapshot(&graph);
        let result = run_finance_sweep(&snapshot);
        assert!(result.error.is_none(), "{:?}", result.error);
        assert_eq!(result.portfolios.len(), 1);
        assert!(result.portfolios[0].final_wealth > 0.0);
        assert!(result.assets_loaded >= 1, "{:?}", result);
    }

    #[test]
    fn bare_ticker_csv_path_uses_bundled_sample_data() {
        let mut warnings = Vec::new();
        let (prices, loaded, synthetic) = load_asset_close_series(
            "SPY",
            Some(&"SPY".to_string()),
            "/MarketLab/Universe/SPY",
            &mut warnings,
        );
        assert!(loaded, "warnings: {warnings:?}");
        assert!(!synthetic);
        assert!(prices.len() >= 2);
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
        let closes = load_close_prices(&path).expect("parse modern yahoo csv");
        assert_eq!(closes.len(), 1);
        assert!((closes[0] - 472.65).abs() < f64::EPSILON);
    }
}
