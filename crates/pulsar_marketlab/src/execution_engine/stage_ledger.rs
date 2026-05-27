//! Continuous-time simulation ledger backed by [`MarketStage`] time samples.

use std::fmt;

use crate::trading_stage::{asset_prim_path, MarketStage, MarketStagePathError};

pub const EXECUTION_CASH_PATH: &str = "/execution/portfolio/cash";
pub const EXECUTION_CASH_ATTR: &str = "balance";
pub const EXECUTION_POSITIONS_PREFIX: &str = "/execution/portfolio/positions";

/// Build `/execution/portfolio/positions/{ticker}`.
pub fn position_prim_path(ticker: &str) -> Result<String, MarketStagePathError> {
    let ticker = ticker.trim();
    if ticker.is_empty() || ticker.contains('/') {
        return Err(MarketStagePathError::InvalidPath);
    }
    let path = format!("{EXECUTION_POSITIONS_PREFIX}/{ticker}");
    crate::trading_stage::validate_stage_path(&path)?;
    Ok(path)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StageLedgerError {
    InvalidPath(MarketStagePathError),
    InvalidTransactionTime,
}

impl fmt::Display for StageLedgerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StageLedgerError::InvalidPath(error) => write!(f, "{error}"),
            StageLedgerError::InvalidTransactionTime => {
                write!(f, "transaction time must be finite")
            }
        }
    }
}

impl std::error::Error for StageLedgerError {}

impl From<MarketStagePathError> for StageLedgerError {
    fn from(value: MarketStagePathError) -> Self {
        Self::InvalidPath(value)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SimulationTransaction {
    pub time: f64,
    pub cash_delta: f64,
    pub position_deltas: Vec<(String, f64)>,
}

pub struct StageSimulationLedger;

impl StageSimulationLedger {
    pub fn reset_execution_paths(stage: &mut MarketStage) {
        stage
            .prims
            .retain(|path, _| !path.starts_with("/execution/"));
    }

    pub fn seed_initial_cash(
        stage: &mut MarketStage,
        initial_cash: f64,
    ) -> Result<(), StageLedgerError> {
        if !initial_cash.is_finite() {
            return Err(StageLedgerError::InvalidTransactionTime);
        }
        stage
            .set_sample(EXECUTION_CASH_PATH, EXECUTION_CASH_ATTR, 0.0, initial_cash as f32)
            .map_err(StageLedgerError::from)
    }

    pub fn cash_at(stage: &MarketStage, t: f64) -> f64 {
        stage
            .resolve_attribute_at(EXECUTION_CASH_PATH, EXECUTION_CASH_ATTR, t)
            .map(f64::from)
            .unwrap_or(0.0)
    }

    pub fn shares_at(stage: &MarketStage, ticker: &str, t: f64) -> f64 {
        position_prim_path(ticker)
            .ok()
            .and_then(|path| stage.resolve_attribute_at(&path, "shares", t))
            .map(f64::from)
            .unwrap_or(0.0)
    }

    pub fn mark_price_at(stage: &MarketStage, ticker: &str, t: f64) -> Option<f64> {
        asset_prim_path(ticker)
            .ok()
            .and_then(|path| stage.resolve_attribute_at(&path, "close", t))
            .map(f64::from)
            .filter(|price| price.is_finite() && *price > 0.0)
    }

    pub fn apply_transaction(
        stage: &mut MarketStage,
        tx: &SimulationTransaction,
    ) -> Result<(), StageLedgerError> {
        if !tx.time.is_finite() {
            return Err(StageLedgerError::InvalidTransactionTime);
        }
        let cash = Self::cash_at(stage, tx.time) + tx.cash_delta;
        stage
            .set_sample(
                EXECUTION_CASH_PATH,
                EXECUTION_CASH_ATTR,
                tx.time,
                cash as f32,
            )
            .map_err(StageLedgerError::from)?;

        for (ticker, delta) in &tx.position_deltas {
            let path = position_prim_path(ticker)?;
            let shares = Self::shares_at(stage, ticker, tx.time) + delta;
            stage
                .set_sample(&path, "shares", tx.time, shares as f32)
                .map_err(StageLedgerError::from)?;
        }
        Ok(())
    }

    /// `NAV(t) = CashBalance(t) + Σ Shares_i(t) × MarkPrice_i(t)`.
    pub fn nav_at_time(stage: &MarketStage, t: f64, tickers: &[&str]) -> f64 {
        if !t.is_finite() {
            return 0.0;
        }
        let cash = Self::cash_at(stage, t);
        let positions: f64 = tickers
            .iter()
            .map(|ticker| {
                let shares = Self::shares_at(stage, ticker, t);
                let mark = Self::mark_price_at(stage, ticker, t).unwrap_or(0.0);
                shares * mark
            })
            .sum();
        cash + positions
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trading_stage::MarketStage;

    #[test]
    fn seeds_cash_at_origin() {
        let mut stage = MarketStage::new();
        StageSimulationLedger::seed_initial_cash(&mut stage, 10_000.0).unwrap();
        assert_eq!(StageSimulationLedger::cash_at(&stage, 0.0), 10_000.0);
        assert_eq!(StageSimulationLedger::cash_at(&stage, 5.0), 10_000.0);
    }

    #[test]
    fn buy_transaction_updates_cash_and_shares_at_time() {
        let mut stage = MarketStage::new();
        StageSimulationLedger::seed_initial_cash(&mut stage, 1_000.0).unwrap();
        stage
            .set_sample("/assets/SPY", "close", 100.0, 50.0)
            .unwrap();
        StageSimulationLedger::apply_transaction(
            &mut stage,
            &SimulationTransaction {
                time: 100.0,
                cash_delta: -500.0,
                position_deltas: vec![("SPY".into(), 10.0)],
            },
        )
        .unwrap();
        assert_eq!(StageSimulationLedger::cash_at(&stage, 100.0), 500.0);
        assert_eq!(StageSimulationLedger::shares_at(&stage, "SPY", 100.0), 10.0);
        assert_eq!(StageSimulationLedger::shares_at(&stage, "SPY", 150.0), 10.0);
    }

    #[test]
    fn nav_at_time_uses_forward_filled_inputs() {
        let mut stage = MarketStage::new();
        StageSimulationLedger::seed_initial_cash(&mut stage, 1_000.0).unwrap();
        stage.set_sample("/assets/SPY", "close", 100.0, 50.0).unwrap();
        StageSimulationLedger::apply_transaction(
            &mut stage,
            &SimulationTransaction {
                time: 100.0,
                cash_delta: -500.0,
                position_deltas: vec![("SPY".into(), 10.0)],
            },
        )
        .unwrap();
        assert_eq!(StageSimulationLedger::cash_at(&stage, 120.0), 500.0);
        assert_eq!(StageSimulationLedger::nav_at_time(&stage, 120.0, &["SPY"]), 1_000.0);
    }
}
