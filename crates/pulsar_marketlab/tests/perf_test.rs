//! Headless perf integration tests isolating OTL engine time from USD disk roundtrip overhead.
//!
//! Run: `cargo test --test perf_test -- --nocapture`

use std::collections::HashMap;
use std::io::Write;
use std::sync::Arc;
use std::time::Instant;

use pulsar_marketlab_core::{MarketLabGraphEngine, SharedPriceColumn};
use pulsar_marketlab_ui::workspace::{build_stage_graph_snapshot, WorkspaceContext};

const BAR_COUNT: usize = 2872;
const ASSET_COUNT: usize = 6;

/// Minimal 6-asset equal-weight portfolio stage (in-memory compose equivalent).
const PERF_PIPELINE_USDA: &str = r#"#usda 1.0
(
    defaultPrim = "MarketLab"
)

def Scope "MarketLab"
{
    def Scope "Universe"
    {
        def FinancialAsset "ASSET2" { token inputs:symbol = "SPY" }
        def FinancialAsset "ASSET3" { token inputs:symbol = "ASSET3" }
        def FinancialAsset "ASSET4" { token inputs:symbol = "ASSET4" }
        def FinancialAsset "ASSET5" { token inputs:symbol = "ASSET5" }
        def FinancialAsset "ASSET6" { token inputs:symbol = "ASSET6" }
        def FinancialAsset "ASSET7" { token inputs:symbol = "ASSET7" }
    }
    def Scope "Portfolios"
    {
        def PortfolioIntegrator "Fund"
        {
            token inputs:id = "Allocation::EqualWeight"
            double inputs:initial_capital = 10000
            rel inputs:sources = [
                </MarketLab/Universe/ASSET2>,
                </MarketLab/Universe/ASSET3>,
                </MarketLab/Universe/ASSET4>,
                </MarketLab/Universe/ASSET5>,
                </MarketLab/Universe/ASSET6>,
                </MarketLab/Universe/ASSET7>,
            ]
        }
    }
}
"#;

fn synthetic_series() -> Arc<[f64]> {
    (0..BAR_COUNT)
        .map(|i| 100.0 + i as f64 * 0.05)
        .collect::<Vec<_>>()
        .into()
}

fn asset_vectors() -> HashMap<String, SharedPriceColumn> {
    let series = synthetic_series();
    let paths = [
        "/MarketLab/Universe/ASSET2",
        "/MarketLab/Universe/ASSET3",
        "/MarketLab/Universe/ASSET4",
        "/MarketLab/Universe/ASSET5",
        "/MarketLab/Universe/ASSET6",
        "/MarketLab/Universe/ASSET7",
    ];
    paths
        .into_iter()
        .map(|path| (path.to_string(), SharedPriceColumn::from_series(Arc::clone(&series))))
        .collect()
}

#[test]
fn perf_engine_canvas_direct() {
    let vectors = asset_vectors();
    let started = Instant::now();
    let context = WorkspaceContext::from_usda_text(PERF_PIPELINE_USDA).unwrap_or_default();
    let snapshot = build_stage_graph_snapshot(context.usd_stage());
    let mut engine = MarketLabGraphEngine::compile_from_canvas(&snapshot).expect("compile");
    let _ = engine.execute_timeline(vectors, BAR_COUNT);
    eprintln!(
        "perf_engine_canvas_direct ({}×{}): {} ms",
        ASSET_COUNT,
        BAR_COUNT,
        started.elapsed().as_millis()
    );
}

#[test]
fn perf_engine_usd_roundtrip() {
    let vectors = asset_vectors();
    let started = Instant::now();
    let mut tmp = tempfile::NamedTempFile::new().expect("temp usda");
    write!(tmp, "{PERF_PIPELINE_USDA}").expect("write usda");
    let context =
        WorkspaceContext::new(tmp.path()).expect("open stage from disk");
    let snapshot = build_stage_graph_snapshot(context.usd_stage());
    let mut engine = MarketLabGraphEngine::compile_from_stage(&snapshot).expect("compile");
    let _ = engine.execute_timeline(vectors, BAR_COUNT);
    eprintln!(
        "perf_engine_usd_roundtrip ({}×{}): {} ms",
        ASSET_COUNT,
        BAR_COUNT,
        started.elapsed().as_millis()
    );
}

#[test]
fn perf_usd_compose_only() {
    let started = Instant::now();
    let _ = WorkspaceContext::from_usda_text(PERF_PIPELINE_USDA);
    eprintln!(
        "perf_usd_compose_only ({}×{}): {} ms",
        ASSET_COUNT,
        BAR_COUNT,
        started.elapsed().as_millis()
    );
}

#[test]
fn perf_riskparity_portfolio_dir_optional() {
    let Some(dir) = std::env::var("RISKPARITY_PORTFOLIO_DIR").ok() else {
        eprintln!("RISKPARITY_PORTFOLIO_DIR unset — skipping local profiling fixture");
        return;
    };
    if !std::path::Path::new(&dir).is_dir() {
        eprintln!("RISKPARITY_PORTFOLIO_DIR not a directory — skipping");
        return;
    }
    eprintln!("RISKPARITY_PORTFOLIO_DIR={dir} (fixture hook reserved for future CSV sweep)");
}
