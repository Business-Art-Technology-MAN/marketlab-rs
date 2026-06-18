//! Run a timeline sweep from a compiled [`StageGraphSnapshot`].

use std::collections::HashMap;
use std::sync::Arc;

use pulsar_marketlab_core::{
    MarketLabGraphEngine, SharedPriceColumn, StageGraphPrim, StageGraphSnapshot,
};

use crate::asset_data::load_asset_close_series;
use crate::compile_profile::{finance_compile_profile_to_sweep, FinanceCompileProfile};

const DEFAULT_BAR_COUNT: usize = 252;

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
    run_finance_sweep_with_profile(snapshot, &FinanceCompileProfile::default(), &HashMap::new())
}

/// Execute a sweep with Hydra stage-tree mute / solo overrides.
pub fn run_finance_sweep_with_profile(
    snapshot: &StageGraphSnapshot,
    profile: &FinanceCompileProfile,
    node_prim_paths: &HashMap<String, String>,
) -> FinanceSweepResult {
    let sweep_profile = finance_compile_profile_to_sweep(profile, node_prim_paths);
    run_finance_sweep_internal(snapshot, &sweep_profile)
}

fn run_finance_sweep_internal(
    snapshot: &StageGraphSnapshot,
    sweep_profile: &pulsar_marketlab_core::StageSweepProfile,
) -> FinanceSweepResult {
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
    let result = engine.execute_timeline_with_profile(vectors, timeline_len, sweep_profile);
    let mut portfolios = Vec::new();

    for prim in snapshot
        .prims
        .iter()
        .filter(|prim| prim.type_name == "PortfolioIntegrator")
    {
        if sweep_profile.muted_prim_paths.contains(&prim.path) {
            continue;
        }
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
    fn muted_portfolio_prim_skips_portfolio_sweep_output() {
        use std::collections::HashSet;

        use crate::finance_node_prim_paths;

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
        let paths = finance_node_prim_paths(&graph);
        let profile = FinanceCompileProfile {
            muted_node_ids: HashSet::from(["fund".to_string()]),
            solo_node_id: None,
            node_variant_overrides: HashMap::new(),
        };
        let result = run_finance_sweep_with_profile(&snapshot, &profile, &paths);
        assert!(result.error.is_none(), "{:?}", result.error);
        assert!(result.portfolios.is_empty());
    }

    #[test]
    fn ta_trend_period_is_emitted_in_stage_snapshot_script() {
        let snapshot_with_period = |period: u32| {
            let mut graph = GraphDescription::new("test");
            graph.add_node(NodeInstance::new(
                "spy",
                type_id::FINANCIAL_ASSET,
                Position::new(0.0, 0.0),
            ));
            let mut trend = NodeInstance::new(
                "trend",
                type_id::TA_TREND,
                Position::new(100.0, 0.0),
            );
            trend
                .properties
                .insert("period".into(), graphy::JsonValue::Number(period.into()));
            graph.add_node(trend);
            graph_description_to_stage_snapshot(&graph)
        };

        let short = snapshot_with_period(5);
        let long = snapshot_with_period(120);
        let short_ta = short
            .prims
            .iter()
            .find(|prim| prim.type_name == "OtlTaUberSignal")
            .expect("ta prim");
        let long_ta = long
            .prims
            .iter()
            .find(|prim| prim.type_name == "OtlTaUberSignal")
            .expect("ta prim");
        assert_eq!(short_ta.attributes.get("inputs:period"), Some(&"5".to_string()));
        assert_eq!(long_ta.attributes.get("inputs:period"), Some(&"120".to_string()));
        assert_ne!(
            short_ta.attributes.get("inputs:script_src"),
            long_ta.attributes.get("inputs:script_src")
        );
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
}
