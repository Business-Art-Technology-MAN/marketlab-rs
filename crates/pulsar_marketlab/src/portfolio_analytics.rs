//! Graph-engine portfolio analytics derived from timeline integration output.

use pulsar_marketlab_core::{PortfolioIntegrationResult, PortfolioTrackingFrame};

use crate::workspace_state::PortfolioDiagnosticsSnapshot;

const SHARPE_ANNUALIZATION: f64 = 252.0;
const TRADE_WEIGHT_DELTA_THRESHOLD: f64 = 0.12;

/// Build inspector / canvas diagnostics from a graph-engine portfolio sweep at `playhead`.
pub fn build_portfolio_diagnostics_from_integration(
    integration: &PortfolioIntegrationResult,
    playhead: usize,
    initial_cash: f64,
    simulation_epoch: u64,
    tick_label: Option<String>,
    benchmark_prices: Option<&[f64]>,
) -> PortfolioDiagnosticsSnapshot {
    let bars_processed = integration.wealth_series.len();
    let end = playhead.min(bars_processed.saturating_sub(1));
    let nav_history: Vec<f64> = integration
        .wealth_series
        .iter()
        .take(end + 1)
        .copied()
        .collect();
    let nav = nav_history.last().copied().unwrap_or(initial_cash);

    let exposure_samples = exposure_samples_through_bar(&integration.tracking_matrix, end);
    let trade_count = count_trade_events_through_bar(&integration.tracking_matrix, end);

    let (cash, position_qty, mark_price) =
        ledger_snapshot_at_bar(&integration.tracking_matrix, end, nav);

    let total_return_pct = if initial_cash.abs() > f64::EPSILON {
        (nav - initial_cash) / initial_cash
    } else {
        0.0
    };

    let benchmark_return_pct = benchmark_prices.and_then(|prices| {
        if end + 1 >= 2 && prices.len() > end {
            let first = prices[0];
            let last = prices[end];
            if first.abs() > f64::EPSILON {
                Some((last / first) - 1.0)
            } else {
                None
            }
        } else {
            None
        }
    });
    let excess_return_pct = benchmark_return_pct.map(|benchmark| total_return_pct - benchmark);

    let mut peak_nav = f64::NEG_INFINITY;
    let mut max_drawdown_pct: f64 = 0.0;
    for sample in &nav_history {
        peak_nav = peak_nav.max(*sample);
        if peak_nav > f64::EPSILON {
            max_drawdown_pct = max_drawdown_pct.max((peak_nav - sample) / peak_nav);
        }
    }

    let sharpe_ratio = sharpe_from_nav_history(&nav_history);

    let avg_exposure_pct = if exposure_samples.is_empty() {
        0.0
    } else {
        exposure_samples.iter().sum::<f64>() / exposure_samples.len() as f64
    };

    PortfolioDiagnosticsSnapshot {
        simulation_epoch,
        tick_index: end,
        tick_label,
        nav,
        cash,
        position_qty,
        mark_price,
        total_return_pct,
        max_drawdown_pct,
        sharpe_ratio,
        bars_processed: nav_history.len(),
        trade_count,
        benchmark_return_pct,
        excess_return_pct,
        avg_exposure_pct,
    }
}

fn sharpe_from_nav_history(nav_history: &[f64]) -> Option<f64> {
    if nav_history.len() < 3 {
        return None;
    }
    let returns: Vec<f64> = nav_history
        .windows(2)
        .filter_map(|pair| {
            if pair[0].abs() > f64::EPSILON {
                Some((pair[1] / pair[0]) - 1.0)
            } else {
                None
            }
        })
        .collect();
    if returns.len() < 2 {
        return None;
    }
    let mean = returns.iter().sum::<f64>() / returns.len() as f64;
    let variance = returns
        .iter()
        .map(|sample| {
            let diff = sample - mean;
            diff * diff
        })
        .sum::<f64>()
        / returns.len() as f64;
    let std_dev = variance.sqrt();
    if std_dev > f64::EPSILON {
        Some((mean / std_dev) * SHARPE_ANNUALIZATION.sqrt())
    } else {
        None
    }
}

fn exposure_samples_through_bar(tracking: &[PortfolioTrackingFrame], end: usize) -> Vec<f64> {
    let mut by_bar: std::collections::HashMap<i64, f64> = std::collections::HashMap::new();
    for frame in tracking {
        if frame.timestamp as usize > end {
            continue;
        }
        *by_bar.entry(frame.timestamp).or_insert(0.0) +=
            frame.altered_portfolio_weight.abs();
    }
    let mut bars: Vec<i64> = by_bar.keys().copied().collect();
    bars.sort_unstable();
    bars.into_iter().map(|bar| by_bar[&bar]).collect()
}

fn count_trade_events_through_bar(tracking: &[PortfolioTrackingFrame], end: usize) -> u32 {
    let mut by_bar: std::collections::HashMap<i64, f64> = std::collections::HashMap::new();
    for frame in tracking {
        if frame.timestamp as usize > end {
            continue;
        }
        *by_bar.entry(frame.timestamp).or_insert(0.0) +=
            frame.closure_raw_weight.abs();
    }
    let mut bars: Vec<i64> = by_bar.keys().copied().collect();
    bars.sort_unstable();

    let mut trades = 0_u32;
    let mut prior: Option<f64> = None;
    for bar in bars {
        let weight = by_bar[&bar];
        if let Some(previous) = prior {
            if (weight - previous).abs() >= TRADE_WEIGHT_DELTA_THRESHOLD {
                trades += 1;
            }
        }
        prior = Some(weight);
    }
    trades
}

fn ledger_snapshot_at_bar(
    tracking: &[PortfolioTrackingFrame],
    end: usize,
    nav: f64,
) -> (f64, f64, f64) {
    let leg_frames: Vec<_> = tracking
        .iter()
        .filter(|frame| frame.timestamp as usize == end)
        .collect();
    if leg_frames.is_empty() {
        return (nav, 0.0, 0.0);
    }
    let invested: f64 = leg_frames
        .iter()
        .map(|frame| {
            frame.calculated_units * frame.current_nominal_price * frame.closure_raw_weight.signum()
        })
        .sum();
    let position_qty: f64 = leg_frames.iter().map(|frame| frame.calculated_units).sum();
    let mark_price = leg_frames
        .iter()
        .map(|frame| frame.current_nominal_price)
        .find(|price| *price > f64::EPSILON)
        .unwrap_or(0.0);
    let cash = (nav - invested).max(0.0);
    (cash, position_qty, mark_price)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pulsar_marketlab_core::PortfolioTrackingFrame;

    #[test]
    fn diagnostics_track_rising_nav() {
        let integration = PortfolioIntegrationResult {
            wealth_series: vec![10_000.0, 10_100.0, 10_250.0],
            tracking_matrix: vec![
                PortfolioTrackingFrame {
                    timestamp: 0,
                    asset_id: "SPY".into(),
                    closure_raw_weight: 1.0,
                    altered_portfolio_weight: 1.0,
                    current_nominal_price: 100.0,
                    calculated_units: 100.0,
                    investment_return: 0.0,
                },
                PortfolioTrackingFrame {
                    timestamp: 2,
                    asset_id: "SPY".into(),
                    closure_raw_weight: 1.0,
                    altered_portfolio_weight: 1.0,
                    current_nominal_price: 102.5,
                    calculated_units: 100.0,
                    investment_return: 0.025,
                },
            ],
        };
        let snapshot = build_portfolio_diagnostics_from_integration(
            &integration,
            2,
            10_000.0,
            0,
            Some("bar 3/3".into()),
            Some(&[100.0, 101.0, 102.5]),
        );
        assert!(snapshot.total_return_pct > 0.0);
        assert_eq!(snapshot.bars_processed, 3);
    }
}
