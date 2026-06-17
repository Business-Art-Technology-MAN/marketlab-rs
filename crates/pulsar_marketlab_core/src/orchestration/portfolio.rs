//! Portfolio integrator: symbolic OTL closure ingestion, base allocation, and tracking matrix.

use std::collections::HashMap;
use std::sync::Arc;

/// Directional exposure encoded in an upstream OTL closure token.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DirectionalDistribution {
    MarketLong,
    MarketShort,
    Neutral,
}

impl DirectionalDistribution {
    pub fn sign(self) -> f64 {
        match self {
            Self::MarketLong => 1.0,
            Self::MarketShort => -1.0,
            Self::Neutral => 0.0,
        }
    }
}

/// Whether an upstream leg is a tradable asset quote or a child portfolio NAV stream.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ClosureLegKind {
    #[default]
    Asset,
    SubPortfolio,
}

/// Symbolic closure token from an upstream OTL/asset node (unitless strategy weight).
#[derive(Clone, Debug, PartialEq)]
pub struct SymbolicOtlClosure {
    pub asset_id: String,
    pub direction: DirectionalDistribution,
    /// Baseline alpha / conviction before portfolio OTL modification (unitless).
    pub closure_raw_weight: f64,
    /// Per-bar OTL signal used to derive direction and optional weight scaling.
    pub signal_series: Vec<f64>,
    pub leg_kind: ClosureLegKind,
}

/// Absolute market quote for nominal sizing (price × multiplier → dollars per unit).
#[derive(Clone, Debug, PartialEq)]
pub struct AssetQuote {
    pub price_series: Arc<[f64]>,
    pub contract_multiplier: f64,
}

impl Default for AssetQuote {
    fn default() -> Self {
        Self {
            price_series: Arc::from([1.0]),
            contract_multiplier: 1.0,
        }
    }
}

impl AssetQuote {
    pub fn price_at(&self, bar_index: usize) -> f64 {
        self.price_series
            .get(bar_index)
            .copied()
            .filter(|value| value.is_finite())
            .unwrap_or(0.0)
    }

    /// Strict lookback: indices before frame `0` resolve to `0.0`.
    pub fn price_at_frame(&self, frame: isize) -> f64 {
        if frame < 0 {
            return 0.0;
        }
        self.price_at(frame as usize)
    }
}

/// Baseline cash allocation mapped from closure weights before OTL modification.
#[derive(Clone, Debug, PartialEq)]
pub struct BasePosition {
    pub asset_id: String,
    pub direction: DirectionalDistribution,
    pub closure_raw_weight: f64,
    pub altered_portfolio_weight: f64,
    pub cash_allocation: f64,
    pub nominal_price: f64,
    pub contract_multiplier: f64,
    pub nominal_units: f64,
    pub prior_nominal_units: f64,
    pub leg_kind: ClosureLegKind,
}

/// One row in the portfolio tracking matrix at a playhead step.
#[derive(Clone, Debug, PartialEq)]
pub struct PortfolioTrackingFrame {
    pub timestamp: i64,
    pub asset_id: String,
    pub closure_raw_weight: f64,
    pub altered_portfolio_weight: f64,
    pub current_nominal_price: f64,
    pub calculated_units: f64,
    pub investment_return: f64,
}

/// Full timeline result from [`integrate_portfolio`].
#[derive(Clone, Debug, PartialEq)]
pub struct PortfolioIntegrationResult {
    pub wealth_series: Vec<f64>,
    pub tracking_matrix: Vec<PortfolioTrackingFrame>,
}

/// Mutable portfolio state passed into an optional OTL modification hook.
#[derive(Debug)]
pub struct PortfolioOtlState<'a> {
    pub bar_index: usize,
    pub timestamp: i64,
    pub total_equity: f64,
    pub peak_equity: f64,
    pub drawdown: f64,
    pub allocation_method: &'a str,
    pub otl_script: &'a str,
    pub positions: &'a mut [BasePosition],
}

pub type PortfolioOtlTransformFn = dyn Fn(PortfolioOtlState<'_>) + Send + Sync;

#[derive(Clone, Debug)]
pub struct PortfolioIntegratorConfig {
    pub allocation_method: String,
    pub initial_capital: f64,
    pub otl_script: String,
}

pub fn normalize_asset_quote_key(asset_id: &str) -> String {
    let leaf = asset_id
        .rsplit('/')
        .next()
        .filter(|leaf| !leaf.is_empty())
        .unwrap_or(asset_id);
    leaf.trim_end_matches(".csv").to_string()
}

fn resolve_asset_quote<'a>(quotes: &'a HashMap<String, AssetQuote>, asset_id: &str) -> AssetQuote {
    if let Some(quote) = quotes.get(asset_id) {
        return quote.clone();
    }
    let key = normalize_asset_quote_key(asset_id);
    if let Some(quote) = quotes.get(&key) {
        return quote.clone();
    }
    quotes
        .iter()
        .find(|(path, _)| {
            path.eq_ignore_ascii_case(asset_id)
                || normalize_asset_quote_key(path) == key
        })
        .map(|(_, quote)| quote.clone())
        .unwrap_or_default()
}

/// Build symbolic closure tokens from upstream scalar series (one token per upstream leg).
///
/// Baseline weights are equal placeholders; [`integrate_portfolio`] applies the selected
/// allocation method at bar 0 using quote history.
pub fn closures_from_upstream_legs(
    legs: &[(String, Vec<f64>, ClosureLegKind)],
    _allocation_method: &str,
) -> Vec<SymbolicOtlClosure> {
    if legs.is_empty() {
        return Vec::new();
    }

    let n = legs.len() as f64;
    legs.iter()
        .map(|(asset_id, series, leg_kind)| {
            let direction = match leg_kind {
                ClosureLegKind::SubPortfolio => DirectionalDistribution::MarketLong,
                ClosureLegKind::Asset => direction_from_series(series),
            };
            SymbolicOtlClosure {
                asset_id: asset_id.clone(),
                direction,
                closure_raw_weight: 1.0 / n,
                signal_series: series.clone(),
                leg_kind: *leg_kind,
            }
        })
        .collect()
}

fn log_returns_from_series(series: &[f64]) -> Vec<f64> {
    series
        .windows(2)
        .filter_map(|window| {
            let prev = window[0];
            let next = window[1];
            if prev.is_finite() && next.is_finite() && prev.abs() > f64::EPSILON {
                Some((next / prev).ln())
            } else {
                None
            }
        })
        .collect()
}

fn mean_and_std(samples: &[f64]) -> (f64, f64) {
    if samples.is_empty() {
        return (0.0, 1.0);
    }
    let mean = samples.iter().sum::<f64>() / samples.len() as f64;
    let variance =
        samples.iter().map(|sample| (sample - mean).powi(2)).sum::<f64>() / samples.len() as f64;
    (mean, variance.sqrt().max(1e-6))
}

fn leg_return_stats(
    closure: &SymbolicOtlClosure,
    quotes: &HashMap<String, AssetQuote>,
) -> (f64, f64) {
    let series = match closure.leg_kind {
        ClosureLegKind::SubPortfolio => closure.signal_series.as_slice(),
        ClosureLegKind::Asset => {
            let quote = resolve_asset_quote(quotes, &closure.asset_id);
            return mean_and_std(&log_returns_from_series(&quote.price_series));
        }
    };
    mean_and_std(&log_returns_from_series(series))
}

/// Compute normalized portfolio weights for the selected allocation method.
pub fn compute_allocation_weights(
    method: &str,
    closures: &[SymbolicOtlClosure],
    quotes: &HashMap<String, AssetQuote>,
) -> Vec<f64> {
    let n = closures.len();
    if n == 0 {
        return Vec::new();
    }
    if method.contains("EqualWeight") {
        return vec![1.0 / n as f64; n];
    }

    let stats: Vec<(f64, f64)> = closures
        .iter()
        .map(|closure| leg_return_stats(closure, quotes))
        .collect();

    if method.contains("HierarchicalRiskParity") {
        let inv_vol: Vec<f64> = stats.iter().map(|(_, vol)| 1.0 / vol).collect();
        let sum: f64 = inv_vol.iter().sum();
        if sum > f64::EPSILON {
            return inv_vol.iter().map(|weight| weight / sum).collect();
        }
    } else if method.contains("MeanVariance") {
        let scores: Vec<f64> = stats
            .iter()
            .map(|(mean, vol)| (mean / (vol * vol)).max(0.0))
            .collect();
        let sum: f64 = scores.iter().sum();
        if sum > f64::EPSILON {
            return scores.iter().map(|weight| weight / sum).collect();
        }
    }

    vec![1.0 / n as f64; n]
}

fn apply_allocation_weights(
    positions: &mut [BasePosition],
    closures: &[SymbolicOtlClosure],
    method: &str,
    quotes: &HashMap<String, AssetQuote>,
    total_equity: f64,
) {
    let weights = compute_allocation_weights(method, closures, quotes);
    for (position, weight) in positions.iter_mut().zip(weights.iter()) {
        position.closure_raw_weight = *weight;
        position.altered_portfolio_weight = *weight * position.direction.sign();
        position.cash_allocation = total_equity * position.altered_portfolio_weight;
        if position.leg_kind == ClosureLegKind::SubPortfolio {
            position.nominal_units = *weight;
        } else {
            let denom = position.nominal_price * position.contract_multiplier;
            if denom > 0.0 {
                position.nominal_units = (total_equity * *weight) / denom;
            }
        }
    }
}

fn direction_from_series(series: &[f64]) -> DirectionalDistribution {
    if series.is_empty() {
        return DirectionalDistribution::MarketLong;
    }
    if let Some(sample) = series.iter().rev().find(|value| **value != 0.0) {
        if *sample > 0.0 {
            DirectionalDistribution::MarketLong
        } else {
            DirectionalDistribution::MarketShort
        }
    } else {
        // TA warmup bars emit zeros — still treat wired legs as deployable long exposure.
        DirectionalDistribution::MarketLong
    }
}

/// Map unitless closure weights to cash and unclipped nominal units at `bar_index`.
pub fn map_closures_to_base_positions(
    closures: &[SymbolicOtlClosure],
    quotes: &HashMap<String, AssetQuote>,
    total_equity: f64,
    bar_index: usize,
) -> Vec<BasePosition> {
    closures
        .iter()
        .map(|closure| {
            let quote = resolve_asset_quote(quotes, &closure.asset_id);
            let price = match closure.leg_kind {
                ClosureLegKind::SubPortfolio => sub_portfolio_nav_at(&closure.signal_series, bar_index),
                ClosureLegKind::Asset => quote.price_at(bar_index),
            };
            let multiplier = quote.contract_multiplier.max(f64::MIN_POSITIVE);
            let directional_weight = closure.closure_raw_weight * closure.direction.sign();
            let cash_allocation = total_equity * directional_weight;
            let nominal_units = if closure.leg_kind == ClosureLegKind::SubPortfolio {
                closure.closure_raw_weight
            } else {
                (total_equity * closure.closure_raw_weight) / (price * multiplier)
            };

            BasePosition {
                asset_id: closure.asset_id.clone(),
                direction: closure.direction,
                closure_raw_weight: closure.closure_raw_weight,
                altered_portfolio_weight: directional_weight,
                cash_allocation,
                nominal_price: price,
                contract_multiplier: multiplier,
                nominal_units,
                prior_nominal_units: 0.0,
                leg_kind: closure.leg_kind,
            }
        })
        .collect()
}

fn sub_portfolio_nav_at(series: &[f64], bar_index: usize) -> f64 {
    series
        .get(bar_index)
        .copied()
        .or_else(|| series.last().copied())
        .filter(|value| value.is_finite() && *value > 0.0)
        .unwrap_or(f64::MIN_POSITIVE)
}

/// Default OTL modification: apply drawdown-aware risk scaling when script mentions drawdown.
pub fn default_portfolio_otl_hook(state: PortfolioOtlState<'_>) {
    let risk_scale = if state.otl_script.contains("drawdown") {
        (1.0 - state.drawdown * 0.5).clamp(0.1, 1.0)
    } else {
        1.0
    };

    for position in state.positions.iter_mut() {
        let scaled = position.closure_raw_weight * risk_scale;
        position.altered_portfolio_weight = scaled * position.direction.sign();
        position.cash_allocation = state.total_equity * position.altered_portfolio_weight;
        if position.leg_kind == ClosureLegKind::SubPortfolio {
            position.nominal_units = scaled;
        } else {
            let denom = position.nominal_price * position.contract_multiplier;
            position.nominal_units = if denom > 0.0 {
                (state.total_equity * scaled) / denom
            } else {
                0.0
            };
        }
    }
}

fn investment_return(
    prior_units: f64,
    current_units: f64,
    prior_price: f64,
    current_price: f64,
    multiplier: f64,
) -> f64 {
    let prior_value = prior_units * prior_price * multiplier;
    if prior_value.abs() <= f64::EPSILON {
        return 0.0;
    }
    let current_value = current_units * current_price * multiplier;
    (current_value - prior_value) / prior_value
}

/// Execute portfolio integration with closure ingestion, base mapping, OTL hook, and tracking matrix.
pub fn integrate_portfolio(
    closures: &[SymbolicOtlClosure],
    quotes: &HashMap<String, AssetQuote>,
    timeline_len: usize,
    config: &PortfolioIntegratorConfig,
    otl_hook: Option<&PortfolioOtlTransformFn>,
) -> PortfolioIntegrationResult {
    let timeline_len = timeline_len.max(1);
    let hook = otl_hook.unwrap_or(&default_portfolio_otl_hook);

    let mut wealth = config.initial_capital;
    let mut peak_equity = config.initial_capital;
    let mut wealth_series = Vec::with_capacity(timeline_len);
    let mut tracking_matrix = Vec::new();
    let mut held_asset_units: HashMap<String, f64> = HashMap::new();
    let mut sub_portfolio_weights: HashMap<String, f64> = HashMap::new();
    let mut prior_leg_prices: HashMap<String, f64> = HashMap::new();

    for bar in 0..timeline_len {
        let timestamp = bar as i64;
        let bar_closures: Vec<SymbolicOtlClosure> = closures
            .iter()
            .map(|closure| {
                let signal = closure
                    .signal_series
                    .get(bar)
                    .copied()
                    .or_else(|| closure.signal_series.last().copied())
                    .unwrap_or(0.0);
                let mut next = closure.clone();
                next.direction = if signal > 0.0 {
                    DirectionalDistribution::MarketLong
                } else if signal < 0.0 {
                    DirectionalDistribution::MarketShort
                } else {
                    // Preserve baseline leg direction during TA warmup bars (signal == 0).
                    closure.direction
                };
                next
            })
            .collect();

        if bar_closures.is_empty() {
            wealth_series.push(wealth);
            continue;
        }

        if bar == 0 {
            let sizing_equity = config.initial_capital;
            let mut positions =
                map_closures_to_base_positions(&bar_closures, quotes, sizing_equity, bar);
            apply_allocation_weights(
                &mut positions,
                &bar_closures,
                &config.allocation_method,
                quotes,
                sizing_equity,
            );

            let drawdown = 0.0;
            hook(PortfolioOtlState {
                bar_index: bar,
                timestamp,
                total_equity: sizing_equity,
                peak_equity,
                drawdown,
                allocation_method: &config.allocation_method,
                positions: &mut positions,
                otl_script: config.otl_script.as_str(),
            });

            held_asset_units.clear();
            sub_portfolio_weights.clear();
            for (position, closure) in positions.iter().zip(bar_closures.iter()) {
                match closure.leg_kind {
                    ClosureLegKind::SubPortfolio => {
                        sub_portfolio_weights
                            .insert(position.asset_id.clone(), position.closure_raw_weight);
                    }
                    ClosureLegKind::Asset => {
                        held_asset_units
                            .insert(position.asset_id.clone(), position.nominal_units);
                    }
                }
            }
        }

        let mut marked_wealth = 0.0;
        for closure in &bar_closures {
            let (current_price, inv_return, leg_wealth, units, weight) = match closure.leg_kind {
                ClosureLegKind::SubPortfolio => {
                    let weight = sub_portfolio_weights
                        .get(&closure.asset_id)
                        .copied()
                        .unwrap_or(closure.closure_raw_weight);
                    let child_nav = sub_portfolio_nav_at(&closure.signal_series, bar);
                    let prior_nav = prior_leg_prices
                        .get(&closure.asset_id)
                        .copied()
                        .unwrap_or(child_nav);
                    let inv_return = if prior_nav > f64::EPSILON {
                        (child_nav - prior_nav) / prior_nav
                    } else {
                        0.0
                    };
                    (child_nav, inv_return, child_nav * weight, weight, weight)
                }
                ClosureLegKind::Asset => {
                    let quote = resolve_asset_quote(quotes, &closure.asset_id);
                    let current_price = quote.price_at(bar);
                    let units = held_asset_units
                        .get(&closure.asset_id)
                        .copied()
                        .unwrap_or(0.0);
                    let prior_price = prior_leg_prices
                        .get(&closure.asset_id)
                        .copied()
                        .unwrap_or(current_price);
                    let inv_return = investment_return(
                        units,
                        units,
                        prior_price,
                        current_price,
                        quote.contract_multiplier,
                    );
                    let leg_wealth =
                        units * current_price * quote.contract_multiplier.max(f64::MIN_POSITIVE);
                    let weight = if wealth > f64::EPSILON {
                        leg_wealth / wealth
                    } else {
                        closure.closure_raw_weight
                    };
                    (current_price, inv_return, leg_wealth, units, weight)
                }
            };

            marked_wealth += leg_wealth;

            tracking_matrix.push(PortfolioTrackingFrame {
                timestamp,
                asset_id: closure.asset_id.clone(),
                closure_raw_weight: weight,
                altered_portfolio_weight: weight * closure.direction.sign(),
                current_nominal_price: current_price,
                calculated_units: units,
                investment_return: inv_return,
            });

            prior_leg_prices.insert(closure.asset_id.clone(), current_price);
        }

        if marked_wealth > f64::EPSILON {
            wealth = marked_wealth;
        }
        peak_equity = peak_equity.max(wealth);
        wealth_series.push(wealth);
    }

    PortfolioIntegrationResult {
        wealth_series,
        tracking_matrix,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nominal_units_respect_price_and_multiplier() {
        let closures = vec![SymbolicOtlClosure {
            asset_id: "SPY".to_string(),
            direction: DirectionalDistribution::MarketLong,
            closure_raw_weight: 0.5,
            signal_series: vec![5000.0],
            leg_kind: ClosureLegKind::Asset,
        }];
        let mut quotes = HashMap::new();
        quotes.insert(
            "SPY".to_string(),
            AssetQuote {
                price_series: Arc::from([5000.0]),
                contract_multiplier: 1.0,
            },
        );
        let positions = map_closures_to_base_positions(&closures, &quotes, 1_000_000.0, 0);
        assert_eq!(positions[0].nominal_units, 100.0);

        quotes.insert(
            "ES".to_string(),
            AssetQuote {
                price_series: Arc::from([5000.0]),
                contract_multiplier: 50.0,
            },
        );
        let closures_es = vec![SymbolicOtlClosure {
            asset_id: "ES".to_string(),
            direction: DirectionalDistribution::MarketLong,
            closure_raw_weight: 0.5,
            signal_series: vec![5000.0],
            leg_kind: ClosureLegKind::Asset,
        }];
        let positions_es = map_closures_to_base_positions(&closures_es, &quotes, 1_000_000.0, 0);
        assert!((positions_es[0].nominal_units - 2.0).abs() < 1e-9);
    }

    #[test]
    fn integrate_emits_tracking_matrix_rows() {
        let closures = vec![
            SymbolicOtlClosure {
                asset_id: "SPY".to_string(),
                direction: DirectionalDistribution::MarketLong,
                closure_raw_weight: 0.5,
                signal_series: vec![100.0, 102.0],
                leg_kind: ClosureLegKind::Asset,
            },
            SymbolicOtlClosure {
                asset_id: "QQQ".to_string(),
                direction: DirectionalDistribution::MarketLong,
                closure_raw_weight: 0.5,
                signal_series: vec![200.0, 201.0],
                leg_kind: ClosureLegKind::Asset,
            },
        ];
        let mut quotes = HashMap::new();
        quotes.insert(
            "SPY".to_string(),
            AssetQuote {
                price_series: Arc::from([100.0, 102.0]),
                contract_multiplier: 1.0,
            },
        );
        quotes.insert(
            "QQQ".to_string(),
            AssetQuote {
                price_series: Arc::from([200.0, 201.0]),
                contract_multiplier: 1.0,
            },
        );

        let result = integrate_portfolio(
            &closures,
            &quotes,
            2,
            &PortfolioIntegratorConfig {
                allocation_method: "Allocation::EqualWeight".to_string(),
                initial_capital: 1_000_000.0,
                otl_script: String::new(),
            },
            None,
        );

        assert_eq!(result.wealth_series.len(), 2);
        assert_eq!(result.tracking_matrix.len(), 4);
        assert!(result.tracking_matrix[0].timestamp == 0);
        assert!(!result.tracking_matrix[0].asset_id.is_empty());
    }

    #[test]
    fn integrate_nested_sub_portfolio_legs_track_child_nav() {
        let child_one: Vec<f64> = (0..5).map(|i| 10_000.0 + i as f64 * 100.0).collect();
        let child_two: Vec<f64> = (0..5).map(|i| 10_000.0 + i as f64 * 50.0).collect();
        let closures = vec![
            SymbolicOtlClosure {
                asset_id: "Sim_Portfolio_1".to_string(),
                direction: DirectionalDistribution::MarketLong,
                closure_raw_weight: 0.5,
                signal_series: child_one,
                leg_kind: ClosureLegKind::SubPortfolio,
            },
            SymbolicOtlClosure {
                asset_id: "Sim_Portfolio_2".to_string(),
                direction: DirectionalDistribution::MarketLong,
                closure_raw_weight: 0.5,
                signal_series: child_two,
                leg_kind: ClosureLegKind::SubPortfolio,
            },
        ];

        let result = integrate_portfolio(
            &closures,
            &HashMap::new(),
            5,
            &PortfolioIntegratorConfig {
                allocation_method: "Allocation::EqualWeight".to_string(),
                initial_capital: 10_000.0,
                otl_script: String::new(),
            },
            None,
        );

        assert_eq!(result.wealth_series.first().copied(), Some(10_000.0));
        assert!(
            result.wealth_series.last().copied().unwrap_or(0.0) > 10_250.0,
            "parent NAV should aggregate child growth, got {:?}",
            result.wealth_series
        );
    }

    #[test]
    fn integrate_asset_legs_track_price_moves_with_prim_path_quotes() {
        let prices: Vec<f64> = (0..5).map(|i| 100.0 + i as f64 * 2.0).collect();
        // Closure id may be leaf or prim path; quote map is keyed by absolute prim path (graph sweep).
        let closures = vec![SymbolicOtlClosure {
            asset_id: "QQQ".to_string(),
            direction: DirectionalDistribution::MarketLong,
            closure_raw_weight: 1.0,
            signal_series: prices.clone(),
            leg_kind: ClosureLegKind::Asset,
        }];
        let quotes = HashMap::from([(
            "/MarketLab/QQQ".to_string(),
            AssetQuote {
                price_series: Arc::from(prices.into_boxed_slice()),
                contract_multiplier: 1.0,
            },
        )]);

        let result = integrate_portfolio(
            &closures,
            &quotes,
            5,
            &PortfolioIntegratorConfig {
                allocation_method: "Allocation::EqualWeight".to_string(),
                initial_capital: 10_000.0,
                otl_script: String::new(),
            },
            None,
        );

        assert!(
            result.wealth_series.last().copied().unwrap_or(0.0) > 10_000.0,
            "prim-path keyed quotes should drive rising NAV, got {:?}",
            result.wealth_series
        );
    }

    #[test]
    fn integrate_asset_legs_track_price_moves() {
        let prices: Vec<f64> = (0..5).map(|i| 100.0 + i as f64 * 2.0).collect();
        let closures = vec![SymbolicOtlClosure {
            asset_id: "QQQ".to_string(),
            direction: DirectionalDistribution::MarketLong,
            closure_raw_weight: 1.0,
            signal_series: prices.clone(),
            leg_kind: ClosureLegKind::Asset,
        }];
        let quotes = HashMap::from([(
            "QQQ".to_string(),
            AssetQuote {
                price_series: Arc::from(prices.into_boxed_slice()),
                contract_multiplier: 1.0,
            },
        )]);

        let result = integrate_portfolio(
            &closures,
            &quotes,
            5,
            &PortfolioIntegratorConfig {
                allocation_method: "Allocation::EqualWeight".to_string(),
                initial_capital: 10_000.0,
                otl_script: String::new(),
            },
            None,
        );

        assert!(
            result.wealth_series.last().copied().unwrap_or(0.0) > 10_000.0,
            "asset-backed portfolio should grow with rising prices, got {:?}",
            result.wealth_series
        );
    }

    #[test]
    fn drawdown_script_scales_weights() {
        let closures = vec![SymbolicOtlClosure {
            asset_id: "SPY".to_string(),
            direction: DirectionalDistribution::MarketLong,
            closure_raw_weight: 1.0,
            signal_series: vec![100.0, 100.0],
            leg_kind: ClosureLegKind::Asset,
        }];
        let quotes = HashMap::from([("SPY".to_string(), AssetQuote::default())]);

        let mut positions = map_closures_to_base_positions(&closures, &quotes, 800_000.0, 0);
        default_portfolio_otl_hook(PortfolioOtlState {
            bar_index: 1,
            timestamp: 1,
            total_equity: 800_000.0,
            peak_equity: 1_000_000.0,
            drawdown: 0.2,
            allocation_method: "Allocation::EqualWeight",
            positions: &mut positions,
            otl_script: "reduce on drawdown",
        });
        assert!(positions[0].altered_portfolio_weight < 1.0);
    }

    #[test]
    fn allocation_methods_produce_distinct_weights_for_heterogeneous_volatility() {
        let closures = vec![
            SymbolicOtlClosure {
                asset_id: "LOW_VOL".to_string(),
                direction: DirectionalDistribution::MarketLong,
                closure_raw_weight: 0.5,
                signal_series: vec![100.0; 20],
                leg_kind: ClosureLegKind::Asset,
            },
            SymbolicOtlClosure {
                asset_id: "HIGH_VOL".to_string(),
                direction: DirectionalDistribution::MarketLong,
                closure_raw_weight: 0.5,
                signal_series: (0..20)
                    .map(|i| 100.0 + (i as f64 * 3.0).sin() * 10.0)
                    .collect(),
                leg_kind: ClosureLegKind::Asset,
            },
        ];
        let quotes = HashMap::from([
            (
                "LOW_VOL".to_string(),
                AssetQuote {
                    price_series: Arc::from((0..20).map(|_| 100.0).collect::<Vec<_>>().into_boxed_slice()),
                    contract_multiplier: 1.0,
                },
            ),
            (
                "HIGH_VOL".to_string(),
                AssetQuote {
                    price_series: (0..20)
                        .map(|i| 100.0 + (i as f64 * 3.0).sin() * 10.0)
                        .collect(),
                    contract_multiplier: 1.0,
                },
            ),
        ]);

        let equal = compute_allocation_weights("Allocation::EqualWeight", &closures, &quotes);
        let hrp = compute_allocation_weights("Allocation::HierarchicalRiskParity", &closures, &quotes);
        let mean_var = compute_allocation_weights("Allocation::MeanVariance", &closures, &quotes);

        assert!((equal[0] - 0.5).abs() < 1e-9);
        assert!((equal[1] - 0.5).abs() < 1e-9);
        assert!(hrp[0] > equal[0], "HRP should overweight lower-vol leg");
        assert!(hrp[0] > hrp[1]);
        assert_ne!(hrp, mean_var);
    }

    #[test]
    fn integrate_portfolio_allocation_method_changes_wealth_path() {
        let low_vol_prices: Vec<f64> = (0..10).map(|i| 100.0 + i as f64 * 0.1).collect();
        let high_vol_prices: Vec<f64> = (0..10)
            .map(|i| 100.0 + (i as f64 * 2.5).sin() * 8.0 + i as f64 * 0.05)
            .collect();
        let closures = vec![
            SymbolicOtlClosure {
                asset_id: "LOW_VOL".to_string(),
                direction: DirectionalDistribution::MarketLong,
                closure_raw_weight: 0.5,
                signal_series: low_vol_prices.clone(),
                leg_kind: ClosureLegKind::Asset,
            },
            SymbolicOtlClosure {
                asset_id: "HIGH_VOL".to_string(),
                direction: DirectionalDistribution::MarketLong,
                closure_raw_weight: 0.5,
                signal_series: high_vol_prices.clone(),
                leg_kind: ClosureLegKind::Asset,
            },
        ];
        let quotes = HashMap::from([
            (
                "LOW_VOL".to_string(),
                AssetQuote {
                    price_series: Arc::from(low_vol_prices.into_boxed_slice()),
                    contract_multiplier: 1.0,
                },
            ),
            (
                "HIGH_VOL".to_string(),
                AssetQuote {
                    price_series: Arc::from(high_vol_prices.into_boxed_slice()),
                    contract_multiplier: 1.0,
                },
            ),
        ]);

        let equal = integrate_portfolio(
            &closures,
            &quotes,
            10,
            &PortfolioIntegratorConfig {
                allocation_method: "Allocation::EqualWeight".to_string(),
                initial_capital: 10_000.0,
                otl_script: String::new(),
            },
            None,
        );
        let hrp = integrate_portfolio(
            &closures,
            &quotes,
            10,
            &PortfolioIntegratorConfig {
                allocation_method: "Allocation::HierarchicalRiskParity".to_string(),
                initial_capital: 10_000.0,
                otl_script: String::new(),
            },
            None,
        );

        assert_ne!(
            equal.wealth_series.last(),
            hrp.wealth_series.last(),
            "allocation method should change terminal wealth"
        );
    }
}
