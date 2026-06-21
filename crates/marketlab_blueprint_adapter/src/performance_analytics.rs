//! Rust-native performance metrics (pyfolio / PerformanceAnalytics inspired).

const TRADING_DAYS_PER_YEAR: f64 = 252.0;

#[derive(Clone, Debug, PartialEq)]
pub struct FinancePerformanceSummary {
    pub total_return_pct: f64,
    pub cagr_pct: f64,
    pub ann_volatility_pct: f64,
    pub sharpe: f64,
    pub sortino: f64,
    pub max_drawdown_pct: f64,
    pub calmar: f64,
    pub win_rate_pct: f64,
    pub best_period_pct: f64,
    pub worst_period_pct: f64,
    pub periods_per_year: f64,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct FinanceBenchmarkComparison {
    pub label: String,
    pub total_return_pct: f64,
    pub cumulative_return_pct: Vec<f64>,
    pub alpha_pct: f64,
    pub beta: f64,
    pub correlation: f64,
    pub capture_ratio_up: f64,
    pub capture_ratio_down: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FinancePerformanceSeriesBundle {
    pub wealth: Vec<f64>,
    pub period_returns_pct: Vec<f64>,
    pub cumulative_return_pct: Vec<f64>,
    pub drawdown_pct: Vec<f64>,
    pub rolling_sharpe: Vec<f64>,
    pub rolling_volatility_pct: Vec<f64>,
    pub summary: FinancePerformanceSummary,
}

pub fn periods_per_year_from_bar_count(bar_count: usize) -> f64 {
    if bar_count >= 200 {
        TRADING_DAYS_PER_YEAR
    } else if bar_count >= 50 {
        52.0
    } else {
        (bar_count as f64).max(1.0)
    }
}

pub fn wealth_to_period_returns(wealth: &[f64]) -> Vec<f64> {
    if wealth.len() < 2 {
        return Vec::new();
    }
    wealth
        .windows(2)
        .map(|window| {
            let prev = window[0].abs().max(f64::EPSILON);
            (window[1] / prev - 1.0) * 100.0
        })
        .collect()
}

pub fn cumulative_return_index(wealth: &[f64]) -> Vec<f64> {
    if wealth.is_empty() {
        return Vec::new();
    }
    let base = wealth[0].abs().max(f64::EPSILON);
    wealth.iter().map(|value| (value / base - 1.0) * 100.0).collect()
}

pub fn drawdown_series_pct(wealth: &[f64]) -> Vec<f64> {
    let mut peak = f64::NEG_INFINITY;
    wealth
        .iter()
        .map(|value| {
            peak = peak.max(*value);
            if peak <= f64::EPSILON {
                0.0
            } else {
                (value / peak - 1.0) * 100.0
            }
        })
        .collect()
}

pub fn compute_performance_bundle(
    wealth: &[f64],
    risk_free_rate_annual: f64,
    rolling_window: usize,
    periods_per_year: f64,
) -> Option<FinancePerformanceSeriesBundle> {
    if wealth.len() < 2 {
        return None;
    }
    let period_returns_pct = wealth_to_period_returns(wealth);
    let cumulative_return_pct = cumulative_return_index(wealth);
    let drawdown_pct = drawdown_series_pct(wealth);
    let rolling_sharpe = rolling_sharpe_series(&period_returns_pct, rolling_window, risk_free_rate_annual, periods_per_year);
    let rolling_volatility_pct =
        rolling_volatility_series(&period_returns_pct, rolling_window, periods_per_year);
    let summary = summarize_performance(
        &period_returns_pct,
        &drawdown_pct,
        risk_free_rate_annual,
        periods_per_year,
    );
    Some(FinancePerformanceSeriesBundle {
        wealth: wealth.to_vec(),
        period_returns_pct,
        cumulative_return_pct,
        drawdown_pct,
        rolling_sharpe,
        rolling_volatility_pct,
        summary,
    })
}

pub fn summarize_performance(
    period_returns_pct: &[f64],
    drawdown_pct: &[f64],
    risk_free_rate_annual: f64,
    periods_per_year: f64,
) -> FinancePerformanceSummary {
    let total_return_pct: f64 = period_returns_pct.iter().sum();
    let years = (period_returns_pct.len() as f64 / periods_per_year).max(1.0 / periods_per_year);
    let growth: f64 = 1.0 + total_return_pct / 100.0;
    let cagr_pct = (growth.max(0.0).powf(1.0 / years) - 1.0) * 100.0;
    let ann_volatility_pct = ann_volatility(period_returns_pct, periods_per_year);
    let sharpe = sharpe_ratio(period_returns_pct, risk_free_rate_annual, periods_per_year);
    let sortino = sortino_ratio(period_returns_pct, risk_free_rate_annual, periods_per_year);
    let max_drawdown_pct = drawdown_pct
        .iter()
        .copied()
        .fold(0.0, f64::min)
        .abs();
    let calmar = if max_drawdown_pct > f64::EPSILON {
        cagr_pct / max_drawdown_pct
    } else {
        0.0
    };
    let wins = period_returns_pct.iter().filter(|value| **value > 0.0).count();
    let win_rate_pct = if period_returns_pct.is_empty() {
        0.0
    } else {
        wins as f64 / period_returns_pct.len() as f64 * 100.0
    };
    let best_period_pct = period_returns_pct
        .iter()
        .copied()
        .fold(f64::NEG_INFINITY, f64::max);
    let worst_period_pct = period_returns_pct
        .iter()
        .copied()
        .fold(f64::INFINITY, f64::min);

    FinancePerformanceSummary {
        total_return_pct,
        cagr_pct,
        ann_volatility_pct,
        sharpe,
        sortino,
        max_drawdown_pct,
        calmar,
        win_rate_pct,
        best_period_pct,
        worst_period_pct,
        periods_per_year,
    }
}

pub fn compare_to_benchmark(
    strategy_returns_pct: &[f64],
    benchmark_wealth: &[f64],
    label: &str,
) -> Option<FinanceBenchmarkComparison> {
    if strategy_returns_pct.is_empty() || benchmark_wealth.len() < 2 {
        return None;
    }
    let benchmark_returns = wealth_to_period_returns(benchmark_wealth);
    let len = strategy_returns_pct.len().min(benchmark_returns.len());
    if len < 2 {
        return None;
    }
    let strategy = &strategy_returns_pct[..len];
    let benchmark = &benchmark_returns[..len];
    let cumulative_return_pct = cumulative_return_index(&benchmark_wealth[..=len]);
    let total_return_pct: f64 = benchmark.iter().sum();
    let beta = compute_beta(strategy, benchmark);
    let correlation = pearson_correlation(strategy, benchmark);
    let alpha_pct = strategy.iter().sum::<f64>() - beta * benchmark.iter().sum::<f64>();
    let (capture_ratio_up, capture_ratio_down) = capture_ratios(strategy, benchmark);

    Some(FinanceBenchmarkComparison {
        label: label.to_string(),
        total_return_pct,
        cumulative_return_pct,
        alpha_pct,
        beta,
        correlation,
        capture_ratio_up,
        capture_ratio_down,
    })
}

fn ann_volatility(returns_pct: &[f64], periods_per_year: f64) -> f64 {
    if returns_pct.len() < 2 {
        return 0.0;
    }
    let mean = returns_pct.iter().sum::<f64>() / returns_pct.len() as f64;
    let variance = returns_pct
        .iter()
        .map(|value| {
            let delta = value - mean;
            delta * delta
        })
        .sum::<f64>()
        / (returns_pct.len() as f64 - 1.0);
    variance.sqrt() * periods_per_year.sqrt()
}

fn sharpe_ratio(returns_pct: &[f64], risk_free_annual: f64, periods_per_year: f64) -> f64 {
    if returns_pct.is_empty() {
        return 0.0;
    }
    let rf_per_period = risk_free_annual / periods_per_year;
    let mean = returns_pct.iter().sum::<f64>() / returns_pct.len() as f64;
    let vol = ann_volatility(returns_pct, periods_per_year) / periods_per_year.sqrt();
    if vol <= f64::EPSILON {
        0.0
    } else {
        (mean - rf_per_period) / vol
    }
}

fn sortino_ratio(returns_pct: &[f64], risk_free_annual: f64, periods_per_year: f64) -> f64 {
    if returns_pct.is_empty() {
        return 0.0;
    }
    let rf_per_period = risk_free_annual / periods_per_year;
    let mean = returns_pct.iter().sum::<f64>() / returns_pct.len() as f64;
    let downside: Vec<f64> = returns_pct
        .iter()
        .copied()
        .filter(|value| *value < rf_per_period)
        .map(|value| value - rf_per_period)
        .collect();
    if downside.is_empty() {
        return sharpe_ratio(returns_pct, risk_free_annual, periods_per_year);
    }
    let downside_dev = (downside.iter().map(|value| value * value).sum::<f64>()
        / downside.len() as f64)
        .sqrt()
        * periods_per_year.sqrt();
    if downside_dev <= f64::EPSILON {
        0.0
    } else {
        (mean - rf_per_period) / downside_dev * periods_per_year.sqrt()
    }
}

fn rolling_sharpe_series(
    returns_pct: &[f64],
    window: usize,
    risk_free_annual: f64,
    periods_per_year: f64,
) -> Vec<f64> {
    let window = window.max(2);
    returns_pct
        .iter()
        .enumerate()
        .map(|(index, _)| {
            if index + 1 < window {
                return f64::NAN;
            }
            let slice = &returns_pct[index + 1 - window..=index];
            sharpe_ratio(slice, risk_free_annual, periods_per_year)
        })
        .collect()
}

fn rolling_volatility_series(
    returns_pct: &[f64],
    window: usize,
    periods_per_year: f64,
) -> Vec<f64> {
    let window = window.max(2);
    returns_pct
        .iter()
        .enumerate()
        .map(|(index, _)| {
            if index + 1 < window {
                return f64::NAN;
            }
            let slice = &returns_pct[index + 1 - window..=index];
            ann_volatility(slice, periods_per_year)
        })
        .collect()
}

fn pearson_correlation(left: &[f64], right: &[f64]) -> f64 {
    if left.len() != right.len() || left.is_empty() {
        return 0.0;
    }
    let mean_left = left.iter().sum::<f64>() / left.len() as f64;
    let mean_right = right.iter().sum::<f64>() / right.len() as f64;
    let mut num = 0.0;
    let mut den_left = 0.0;
    let mut den_right = 0.0;
    for (a, b) in left.iter().zip(right) {
        let da = a - mean_left;
        let db = b - mean_right;
        num += da * db;
        den_left += da * da;
        den_right += db * db;
    }
    let den = (den_left * den_right).sqrt();
    if den <= f64::EPSILON {
        0.0
    } else {
        num / den
    }
}

fn compute_beta(strategy: &[f64], benchmark: &[f64]) -> f64 {
    if strategy.len() != benchmark.len() || strategy.is_empty() {
        return 0.0;
    }
    let mean_s = strategy.iter().sum::<f64>() / strategy.len() as f64;
    let mean_b = benchmark.iter().sum::<f64>() / benchmark.len() as f64;
    let mut cov = 0.0;
    let mut var_b = 0.0;
    for (s, b) in strategy.iter().zip(benchmark) {
        let ds = s - mean_s;
        let db = b - mean_b;
        cov += ds * db;
        var_b += db * db;
    }
    if var_b <= f64::EPSILON {
        0.0
    } else {
        cov / var_b
    }
}

fn capture_ratios(strategy: &[f64], benchmark: &[f64]) -> (f64, f64) {
    let mut up_strategy = 0.0;
    let mut up_benchmark = 0.0;
    let mut down_strategy = 0.0;
    let mut down_benchmark = 0.0;
    for (s, b) in strategy.iter().zip(benchmark) {
        if *b > 0.0 {
            up_strategy += s;
            up_benchmark += b;
        } else if *b < 0.0 {
            down_strategy += s;
            down_benchmark += b;
        }
    }
    let up = if up_benchmark.abs() > f64::EPSILON {
        up_strategy / up_benchmark
    } else {
        0.0
    };
    let down = if down_benchmark.abs() > f64::EPSILON {
        down_strategy / down_benchmark
    } else {
        0.0
    };
    (up, down)
}

pub fn align_wealth_series(primary: &[f64], secondary: &[f64]) -> (Vec<f64>, Vec<f64>) {
    let len = primary.len().min(secondary.len());
    if len == 0 {
        return (Vec::new(), Vec::new());
    }
    (primary[..len].to_vec(), secondary[..len].to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn computes_drawdown_and_summary() {
        let wealth = vec![100.0, 110.0, 105.0, 120.0];
        let bundle = compute_performance_bundle(&wealth, 0.0, 2, 252.0).expect("bundle");
        assert!(bundle.summary.total_return_pct > 0.0);
        assert!(bundle.drawdown_pct.iter().any(|value| *value < 0.0));
    }
}
