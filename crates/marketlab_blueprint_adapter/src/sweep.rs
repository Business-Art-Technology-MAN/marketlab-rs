//! Run a timeline sweep from a compiled [`StageGraphSnapshot`].

use std::collections::HashMap;
use std::sync::Arc;

use pulsar_marketlab_core::{
    MarketLabGraphEngine, SharedPriceColumn, StageGraphPrim, StageGraphSnapshot,
};

use crate::asset_data::load_asset_close_series_for_prim;
use crate::compile_profile::{finance_compile_profile_to_sweep, FinanceCompileProfile};
use crate::snapshot::snapshot_for_engine_execution;

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
    /// OTL / TA output series keyed by analytics prim path.
    pub analytics_signals: HashMap<String, Vec<f64>>,
    /// All scalar attribute streams keyed by prim path then attribute name.
    pub attribute_streams: HashMap<String, HashMap<String, Vec<f64>>>,
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
    let execution_snapshot = snapshot_for_engine_execution(snapshot);
    let engine = match MarketLabGraphEngine::compile_from_stage(&execution_snapshot) {
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

    let mut attribute_streams: HashMap<String, HashMap<String, Vec<f64>>> = HashMap::new();
    for stream in &result.streams {
        if stream.values.is_empty() {
            continue;
        }
        attribute_streams
            .entry(stream.prim_path.clone())
            .or_default()
            .insert(stream.attribute.clone(), stream.values.clone());
    }

    let analytics_signals = collect_analytics_signals(snapshot, &attribute_streams);

    FinanceSweepResult {
        timeline_len,
        assets_loaded: load_meta.loaded,
        assets_synthetic: load_meta.synthetic,
        portfolios,
        analytics_signals,
        attribute_streams,
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
        .filter(|prim| crate::types::is_finance_price_asset_stage_type(&prim.type_name))
    {
        let (series, loaded, synthetic) =
            load_asset_close_series_for_prim(prim, &mut meta.warnings);
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

const ANALYTICS_SIGNAL_ATTRIBUTES: &[&str] = &["outputs:result", "outputs:signal"];

fn collect_analytics_signals(
    snapshot: &StageGraphSnapshot,
    attribute_streams: &HashMap<String, HashMap<String, Vec<f64>>>,
) -> HashMap<String, Vec<f64>> {
    let mut analytics_signals = HashMap::new();
    for prim in snapshot.prims.iter().filter(|prim| {
        matches!(
            prim.type_name.as_str(),
            "OtlOperator" | "OtlTaUberSignal"
        )
    }) {
        let Some(attrs) = attribute_streams.get(&prim.path) else {
            continue;
        };
        let series = ANALYTICS_SIGNAL_ATTRIBUTES
            .iter()
            .find_map(|attribute| attrs.get(*attribute))
            .or_else(|| attrs.values().next());
        if let Some(values) = series {
            if !values.is_empty() {
                analytics_signals.insert(prim.path.clone(), values.clone());
            }
        }
    }
    analytics_signals
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
    use crate::asset_data::load_asset_close_series;
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
        assert!(
            result.portfolios[0].return_pct.abs() > f64::EPSILON,
            "TA-gated portfolio should move off initial capital, got {:?}",
            result.portfolios[0]
        );
        assert!(result.assets_loaded >= 1, "{:?}", result);
    }

    #[test]
    fn sweep_runs_ta_through_sub_portfolios_into_master() {
        use graphy::JsonValue;

        let mut graph = GraphDescription::new("test");
        let mut spy = NodeInstance::new(
            "spy",
            type_id::FINANCIAL_ASSET,
            Position::new(0.0, 0.0),
        );
        spy.properties
            .insert("symbol".into(), JsonValue::String("SPY".into()));
        let mut qqq = NodeInstance::new(
            "qqq",
            type_id::FINANCIAL_ASSET,
            Position::new(0.0, 80.0),
        );
        qqq.properties
            .insert("symbol".into(), JsonValue::String("QQQ".into()));
        graph.add_node(spy);
        graph.add_node(qqq);
        graph.add_node(NodeInstance::new(
            "ta_ear",
            type_id::TA_TREND,
            Position::new(120.0, 0.0),
        ));
        graph.add_node(NodeInstance::new(
            "ta_ept",
            type_id::TA_TREND,
            Position::new(120.0, 80.0),
        ));
        let mut ear_fund = NodeInstance::new(
            "ear_fund",
            type_id::PORTFOLIO_INTEGRATOR,
            Position::new(240.0, 0.0),
        );
        ear_fund
            .properties
            .insert("name".into(), JsonValue::String("ear_fund".into()));
        let mut ept_fund = NodeInstance::new(
            "ept_fund",
            type_id::PORTFOLIO_INTEGRATOR,
            Position::new(240.0, 80.0),
        );
        ept_fund
            .properties
            .insert("name".into(), JsonValue::String("ept_fund".into()));
        let mut master = NodeInstance::new(
            "master",
            type_id::PORTFOLIO_INTEGRATOR,
            Position::new(360.0, 40.0),
        );
        master
            .properties
            .insert("name".into(), JsonValue::String("fund".into()));
        graph.add_node(ear_fund);
        graph.add_node(ept_fund);
        graph.add_node(master);

        for (asset, ta) in [("spy", "ta_ear"), ("qqq", "ta_ept")] {
            graph.add_connection(Connection {
                source_node: asset.into(),
                source_pin: "close".into(),
                target_node: ta.into(),
                target_pin: "source_stream".into(),
                connection_type: ConnectionType::Data,
            });
        }
        graph.add_connection(Connection {
            source_node: "ta_ear".into(),
            source_pin: "result".into(),
            target_node: "ear_fund".into(),
            target_pin: "signal_0".into(),
            connection_type: ConnectionType::Data,
        });
        graph.add_connection(Connection {
            source_node: "ta_ept".into(),
            source_pin: "result".into(),
            target_node: "ept_fund".into(),
            target_pin: "signal_0".into(),
            connection_type: ConnectionType::Data,
        });
        graph.add_connection(Connection {
            source_node: "ear_fund".into(),
            source_pin: "wealth".into(),
            target_node: "master".into(),
            target_pin: "signal_0".into(),
            connection_type: ConnectionType::Data,
        });
        graph.add_connection(Connection {
            source_node: "ept_fund".into(),
            source_pin: "wealth".into(),
            target_node: "master".into(),
            target_pin: "signal_1".into(),
            connection_type: ConnectionType::Data,
        });

        let snapshot = graph_description_to_stage_snapshot(&graph);
        let result = run_finance_sweep(&snapshot);
        assert!(result.error.is_none(), "{:?}", result.error);
        assert_eq!(result.portfolios.len(), 3, "{:?}", result.portfolios);

        let master_path = snapshot
            .prims
            .iter()
            .find(|prim| prim.path.ends_with("/fund"))
            .map(|prim| prim.path.clone())
            .expect("master prim");
        let master_summary = result
            .portfolios
            .iter()
            .find(|portfolio| portfolio.prim_path == master_path)
            .expect("master sweep");
        assert!(
            master_summary.return_pct.abs() > f64::EPSILON,
            "nested master fund should move off initial capital, got {:?}",
            master_summary
        );
    }

    #[test]
    fn sweep_runs_sub_portfolio_wealth_through_ta_into_master() {
        use graphy::JsonValue;

        let mut graph = GraphDescription::new("test");
        let mut spy = NodeInstance::new(
            "spy",
            type_id::FINANCIAL_ASSET,
            Position::new(0.0, 0.0),
        );
        spy.properties
            .insert("symbol".into(), JsonValue::String("SPY".into()));
        graph.add_node(spy);
        graph.add_node(NodeInstance::new(
            "ta_ear",
            type_id::TA_TREND,
            Position::new(120.0, 0.0),
        ));
        let mut ear_fund = NodeInstance::new(
            "ear_fund",
            type_id::PORTFOLIO_INTEGRATOR,
            Position::new(240.0, 0.0),
        );
        ear_fund
            .properties
            .insert("name".into(), JsonValue::String("ear_fund".into()));
        graph.add_node(ear_fund);
        graph.add_node(NodeInstance::new(
            "ta_master",
            type_id::TA_TREND,
            Position::new(360.0, 0.0),
        ));
        let mut master = NodeInstance::new(
            "master",
            type_id::PORTFOLIO_INTEGRATOR,
            Position::new(480.0, 0.0),
        );
        master
            .properties
            .insert("name".into(), JsonValue::String("fund".into()));
        graph.add_node(master);

        graph.add_connection(Connection {
            source_node: "spy".into(),
            source_pin: "close".into(),
            target_node: "ta_ear".into(),
            target_pin: "source_stream".into(),
            connection_type: ConnectionType::Data,
        });
        graph.add_connection(Connection {
            source_node: "ta_ear".into(),
            source_pin: "result".into(),
            target_node: "ear_fund".into(),
            target_pin: "signal_0".into(),
            connection_type: ConnectionType::Data,
        });
        graph.add_connection(Connection {
            source_node: "ear_fund".into(),
            source_pin: "wealth".into(),
            target_node: "ta_master".into(),
            target_pin: "source_stream".into(),
            connection_type: ConnectionType::Data,
        });
        graph.add_connection(Connection {
            source_node: "ta_master".into(),
            source_pin: "result".into(),
            target_node: "master".into(),
            target_pin: "signal_0".into(),
            connection_type: ConnectionType::Data,
        });

        let snapshot = graph_description_to_stage_snapshot(&graph);
        assert!(
            snapshot.wires.iter().any(|wire| {
                wire.relationship == "inputs:underlying"
                    && wire.source_prim_path.contains("ear_fund")
                    && wire.target_prim_path.contains("ta_master")
            }),
            "expected portfolio wealth to wire into TA as underlying, got {:?}",
            snapshot.wires
        );

        let result = run_finance_sweep(&snapshot);
        assert!(result.error.is_none(), "{:?}", result.error);
        let master_path = snapshot
            .prims
            .iter()
            .find(|prim| prim.path.ends_with("/fund"))
            .map(|prim| prim.path.clone())
            .expect("master prim");
        let master_summary = result
            .portfolios
            .iter()
            .find(|portfolio| portfolio.prim_path == master_path)
            .expect("master sweep");
        assert!(
            master_summary.return_pct.abs() > f64::EPSILON,
            "portfolio wealth fed through TA should move master fund, got {:?}",
            master_summary
        );
    }

    #[test]
    fn sweep_runs_dual_hrp_sub_funds_through_ta_into_master_hrp() {
        use graphy::JsonValue;

        fn hrp_portfolio(id: &str, y: f64) -> NodeInstance {
            let mut node = NodeInstance::new(
                id,
                type_id::PORTFOLIO_INTEGRATOR,
                Position::new(240.0, y),
            );
            node.properties
                .insert("name".into(), JsonValue::String(id.into()));
            node.properties.insert(
                "allocation_id".into(),
                JsonValue::String("Allocation::HierarchicalRiskParity".into()),
            );
            node
        }

        let mut graph = GraphDescription::new("test");
        for (id, y) in [("spy", 0.0), ("qqq", 40.0), ("iwm", 80.0)] {
            let mut asset = NodeInstance::new(
                id,
                type_id::FINANCIAL_ASSET,
                Position::new(0.0, y),
            );
            asset
                .properties
                .insert("symbol".into(), JsonValue::String(id.into()));
            graph.add_node(asset);
            graph.add_node(NodeInstance::new(
                format!("ta_{id}"),
                type_id::TA_TREND,
                Position::new(120.0, y),
            ));
        }
        graph.add_node(hrp_portfolio("ear_fund", 0.0));
        graph.add_node(hrp_portfolio("ept_fund", 80.0));
        graph.add_node(NodeInstance::new(
            "ta_master_ear",
            type_id::TA_TREND,
            Position::new(360.0, 0.0),
        ));
        graph.add_node(NodeInstance::new(
            "ta_master_ept",
            type_id::TA_TREND,
            Position::new(360.0, 80.0),
        ));
        graph.add_node(hrp_portfolio("fund", 40.0));

        for asset in ["spy", "qqq", "iwm"] {
            graph.add_connection(Connection {
                source_node: asset.into(),
                source_pin: "close".into(),
                target_node: format!("ta_{asset}"),
                target_pin: "source_stream".into(),
                connection_type: ConnectionType::Data,
            });
        }
        graph.add_connection(Connection {
            source_node: "ta_spy".into(),
            source_pin: "result".into(),
            target_node: "ear_fund".into(),
            target_pin: "signal_0".into(),
            connection_type: ConnectionType::Data,
        });
        graph.add_connection(Connection {
            source_node: "ta_qqq".into(),
            source_pin: "result".into(),
            target_node: "ear_fund".into(),
            target_pin: "signal_1".into(),
            connection_type: ConnectionType::Data,
        });
        graph.add_connection(Connection {
            source_node: "ta_iwm".into(),
            source_pin: "result".into(),
            target_node: "ept_fund".into(),
            target_pin: "signal_0".into(),
            connection_type: ConnectionType::Data,
        });
        graph.add_connection(Connection {
            source_node: "ear_fund".into(),
            source_pin: "wealth".into(),
            target_node: "ta_master_ear".into(),
            target_pin: "source_stream".into(),
            connection_type: ConnectionType::Data,
        });
        graph.add_connection(Connection {
            source_node: "ept_fund".into(),
            source_pin: "wealth".into(),
            target_node: "ta_master_ept".into(),
            target_pin: "source_stream".into(),
            connection_type: ConnectionType::Data,
        });
        graph.add_connection(Connection {
            source_node: "ta_master_ear".into(),
            source_pin: "result".into(),
            target_node: "fund".into(),
            target_pin: "signal_0".into(),
            connection_type: ConnectionType::Data,
        });
        graph.add_connection(Connection {
            source_node: "ta_master_ept".into(),
            source_pin: "result".into(),
            target_node: "fund".into(),
            target_pin: "signal_1".into(),
            connection_type: ConnectionType::Data,
        });

        let snapshot = graph_description_to_stage_snapshot(&graph);
        let result = run_finance_sweep(&snapshot);
        assert!(result.error.is_none(), "{:?}", result.error);

        let master_path = snapshot
            .prims
            .iter()
            .find(|prim| prim.path.ends_with("/fund"))
            .map(|prim| prim.path.clone())
            .expect("master prim");
        let master_summary = result
            .portfolios
            .iter()
            .find(|portfolio| portfolio.prim_path == master_path)
            .expect("master sweep");
        assert!(
            master_summary.final_wealth > master_summary.initial_capital * 0.5,
            "master fund should retain meaningful wealth when both HRP sub-funds feed through TA, got {:?}",
            master_summary
        );
        assert!(
            master_summary.return_pct > -50.0,
            "master fund should not collapse to zero wealth, got {:?}",
            master_summary
        );
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
    fn ta_trend_default_emits_ma_crossover_script() {
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
        let snapshot = graph_description_to_stage_snapshot(&graph);
        let ta = snapshot
            .prims
            .iter()
            .find(|prim| prim.type_name == "OtlTaUberSignal")
            .expect("ta prim");
        assert_eq!(
            ta.attributes.get("inputs:script_src"),
            Some(&"ta::spread_sign(ta::sma(input, 10), ta::sma(input, 50))".to_string())
        );
    }

    #[test]
    fn ta_trend_signal_period_affects_script_when_below_period() {
        let snapshot_with = |period: u32, signal_period: u32| {
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
            trend.properties.insert(
                "signal_period".into(),
                graphy::JsonValue::Number(signal_period.into()),
            );
            graph.add_node(trend);
            graph_description_to_stage_snapshot(&graph)
        };

        let inverted = snapshot_with(200, 10);
        let swapped = snapshot_with(10, 200);
        let ta_script = |snapshot: &StageGraphSnapshot| {
            snapshot
                .prims
                .iter()
                .find(|prim| prim.type_name == "OtlTaUberSignal")
                .and_then(|prim| prim.attributes.get("inputs:script_src"))
                .cloned()
        };
        assert_eq!(
            ta_script(&inverted),
            Some("ta::spread_sign(ta::sma(input, 10), ta::sma(input, 200))".to_string())
        );
        assert_eq!(ta_script(&swapped), ta_script(&inverted));
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
    fn ta_trend_signal_follows_wired_asset_prices() {
        use graphy::JsonValue;

        let mut graph = GraphDescription::new("test");
        let mut spy = NodeInstance::new(
            "spy",
            type_id::FINANCIAL_ASSET,
            Position::new(0.0, 0.0),
        );
        spy.properties
            .insert("symbol".into(), JsonValue::String("SPY".into()));
        let mut qqq = NodeInstance::new(
            "qqq",
            type_id::FINANCIAL_ASSET,
            Position::new(0.0, 120.0),
        );
        qqq.properties
            .insert("symbol".into(), JsonValue::String("QQQ".into()));
        graph.add_node(spy);
        graph.add_node(qqq);
        graph.add_node(NodeInstance::new(
            "trend_spy",
            type_id::TA_TREND,
            Position::new(160.0, 0.0),
        ));
        graph.add_node(NodeInstance::new(
            "trend_qqq",
            type_id::TA_TREND,
            Position::new(160.0, 120.0),
        ));
        graph.add_connection(Connection {
            source_node: "spy".into(),
            source_pin: "close".into(),
            target_node: "trend_spy".into(),
            target_pin: "source_stream".into(),
            connection_type: ConnectionType::Data,
        });
        graph.add_connection(Connection {
            source_node: "qqq".into(),
            source_pin: "close".into(),
            target_node: "trend_qqq".into(),
            target_pin: "source_stream".into(),
            connection_type: ConnectionType::Data,
        });

        let snapshot = graph_description_to_stage_snapshot(&graph);
        assert!(
            snapshot.wires.iter().any(|wire| {
                wire.relationship == "inputs:underlying"
                    && wire.source_prim_path.contains("SPY")
                    && wire.target_prim_path.contains("trend_spy")
            }),
            "expected asset→TA underlying wire, got {:?}",
            snapshot.wires
        );

        let result = run_finance_sweep(&snapshot);
        assert!(result.error.is_none(), "{:?}", result.error);

        let spy_path = snapshot
            .prims
            .iter()
            .find(|prim| prim.path.contains("trend_spy"))
            .map(|prim| prim.path.clone())
            .expect("trend_spy prim");
        let qqq_path = snapshot
            .prims
            .iter()
            .find(|prim| prim.path.contains("trend_qqq"))
            .map(|prim| prim.path.clone())
            .expect("trend_qqq prim");

        let spy_signal = result
            .analytics_signals
            .get(&spy_path)
            .expect("spy-linked TA signal");
        let qqq_signal = result
            .analytics_signals
            .get(&qqq_path)
            .expect("qqq-linked TA signal");

        assert!(
            spy_signal.iter().any(|value| value.abs() > f64::EPSILON),
            "wired TA should emit non-zero gate values, got {spy_signal:?}"
        );
        assert_ne!(
            spy_signal, qqq_signal,
            "different upstream assets should produce different TA signals"
        );
    }

    #[test]
    fn unwired_ta_trend_differs_from_wired_asset() {
        let mut graph = GraphDescription::new("test");
        graph.add_node(NodeInstance::new(
            "spy",
            type_id::FINANCIAL_ASSET,
            Position::new(0.0, 0.0),
        ));
        graph.add_node(NodeInstance::new(
            "trend_wired",
            type_id::TA_TREND,
            Position::new(120.0, 0.0),
        ));
        graph.add_node(NodeInstance::new(
            "trend_unwired",
            type_id::TA_TREND,
            Position::new(120.0, 80.0),
        ));
        graph.add_connection(Connection {
            source_node: "spy".into(),
            source_pin: "close".into(),
            target_node: "trend_wired".into(),
            target_pin: "source_stream".into(),
            connection_type: ConnectionType::Data,
        });

        let snapshot = graph_description_to_stage_snapshot(&graph);
        let result = run_finance_sweep(&snapshot);
        assert!(result.error.is_none(), "{:?}", result.error);

        let wired_path = snapshot
            .prims
            .iter()
            .find(|prim| prim.path.contains("trend_wired"))
            .map(|prim| prim.path.clone())
            .expect("wired ta prim");
        let unwired_path = snapshot
            .prims
            .iter()
            .find(|prim| prim.path.contains("trend_unwired"))
            .map(|prim| prim.path.clone())
            .expect("unwired ta prim");

        let wired = result.analytics_signals.get(&wired_path).expect("wired signal");
        let unwired = result
            .analytics_signals
            .get(&unwired_path)
            .expect("unwired signal");

        assert!(
            wired.iter().any(|value| value.abs() > f64::EPSILON),
            "wired TA should use asset prices"
        );
        assert!(
            unwired.iter().all(|value| value.abs() < f64::EPSILON),
            "unwired TA should stay flat without upstream, got {unwired:?}"
        );
        assert_ne!(wired, unwired);
    }

    #[test]
    fn ta_trend_accepts_otl_underlying_pin_alias_in_graph() {
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
        graph.add_connection(Connection {
            source_node: "spy".into(),
            source_pin: "close".into(),
            target_node: "trend".into(),
            target_pin: "underlying".into(),
            connection_type: ConnectionType::Data,
        });

        let snapshot = graph_description_to_stage_snapshot(&graph);
        assert!(
            snapshot.wires.iter().any(|wire| wire.relationship == "inputs:underlying"),
            "OTL pin alias should still produce an underlying wire, got {:?}",
            snapshot.wires
        );
        let result = run_finance_sweep(&snapshot);
        assert!(result.error.is_none(), "{:?}", result.error);
        let ta_path = snapshot
            .prims
            .iter()
            .find(|prim| prim.type_name == "OtlTaUberSignal")
            .map(|prim| prim.path.clone())
            .expect("ta prim");
        let signal = result
            .analytics_signals
            .get(&ta_path)
            .expect("ta signal");
        assert!(signal.iter().any(|value| value.abs() > f64::EPSILON));
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
