//! Tier-2 OTL financial intrinsics and stdlib integrators.

use crate::technical_analysis::MarketSeriesWindow;

use super::services::{MarketProviderServices, OtlClosure};
use super::vector::Vector;

pub fn log_return(closes: &[f64], period: usize) -> Option<f64> {
    if period == 0 || closes.len() <= period {
        return None;
    }
    let current = *closes.last()?;
    let prior = closes[closes.len() - 1 - period];
    if current <= 0.0 || prior <= 0.0 {
        return None;
    }
    let value = (current / prior).ln();
    value.is_finite().then_some(value)
}

pub fn realized_volatility(closes: &[f64], period: usize) -> Option<f64> {
    if period < 2 || closes.len() <= period {
        return None;
    }
    let window = &closes[closes.len() - period - 1..];
    let mut returns = Vec::with_capacity(period);
    for pair in window.windows(2) {
        let left = pair[0];
        let right = pair[1];
        if left <= 0.0 || right <= 0.0 {
            return None;
        }
        let value = (right / left).ln();
        if !value.is_finite() {
            return None;
        }
        returns.push(value);
    }
    if returns.len() < 2 {
        return None;
    }
    let mean = returns.iter().sum::<f64>() / returns.len() as f64;
    let variance = returns
        .iter()
        .map(|value| {
            let delta = value - mean;
            delta * delta
        })
        .sum::<f64>()
        / (returns.len() - 1) as f64;
    let value = variance.sqrt();
    value.is_finite().then_some(value)
}

pub fn parkinson_volatility(highs: &[f64], lows: &[f64], period: usize) -> Option<f64> {
    if period == 0 || highs.len() < period || lows.len() < period {
        return None;
    }
    let highs = &highs[highs.len() - period..];
    let lows = &lows[lows.len() - period..];
    let mut sum = 0.0;
    for index in 0..period {
        let high = highs[index];
        let low = lows[index];
        if high <= 0.0 || low <= 0.0 || high < low {
            return None;
        }
        let ratio = (high / low).ln();
        if !ratio.is_finite() {
            return None;
        }
        sum += ratio * ratio;
    }
    let denom = 4.0 * period as f64 * 2.0_f64.ln();
    if denom <= f64::EPSILON {
        return None;
    }
    let value = (sum / denom).sqrt();
    value.is_finite().then_some(value)
}

fn resolve_period(inputs: &[OtlClosure], services: &dyn MarketProviderServices, t: f64) -> Option<usize> {
    Some(
        inputs
            .first()?
            .clone()(services, t)?
            .as_scalar()?
            .round()
            .max(1.0) as usize,
    )
}

fn build_window_from_services(
    services: &dyn MarketProviderServices,
    close_path: &str,
    t: f64,
    bar_count: usize,
) -> Option<MarketSeriesWindow> {
    if bar_count == 0 {
        return None;
    }
    let start = t - bar_count as f64 + 1.0;
    let closes = services.sample_timeline(close_path, start, t);
    if closes.is_empty() {
        return None;
    }
    let mut window = MarketSeriesWindow::default();
    for (offset, close) in closes.iter().enumerate() {
        let time = start + offset as f64;
        let close = close.as_scalar()?;
        let open = services
            .get_global_attribute("open", time)
            .and_then(|value| value.as_scalar())
            .unwrap_or(close);
        let high = services
            .get_global_attribute("high", time)
            .and_then(|value| value.as_scalar())
            .unwrap_or(close);
        let low = services
            .get_global_attribute("low", time)
            .and_then(|value| value.as_scalar())
            .unwrap_or(close);
        let volume = services
            .get_global_attribute("volume", time)
            .and_then(|value| value.as_scalar())
            .unwrap_or(0.0);
        window.push_bar(open, high, low, close, volume);
    }
    Some(window)
}

pub fn execute_financial_integrator(
    services: &dyn MarketProviderServices,
    integrator_name: &str,
    close_path: &str,
    inputs: &[OtlClosure],
    t: f64,
) -> Option<Vector> {
    let period = resolve_period(inputs, services, t)?;
    match integrator_name {
        "financial::rtn_log" => {
            let window = build_window_from_services(services, close_path, t, period + 1)?;
            log_return(&window.close, period).map(Vector::scalar)
        }
        "financial::vol_realized" => {
            let window = build_window_from_services(services, close_path, t, period + 1)?;
            realized_volatility(&window.close, period).map(Vector::scalar)
        }
        "financial::vol_parkinson" => {
            let window = build_window_from_services(services, close_path, t, period)?;
            parkinson_volatility(&window.high, &window.low, period).map(Vector::scalar)
        }
        _ => None,
    }
}

pub fn execute_stdlib_integrator(
    services: &dyn MarketProviderServices,
    integrator_name: &str,
    inputs: &[OtlClosure],
    t: f64,
) -> Option<Vector> {
    match integrator_name {
        "stdlib::clamp" => {
            if inputs.len() != 3 {
                return None;
            }
            let value = inputs[0].clone()(services, t)?.as_scalar()?;
            let min = inputs[1].clone()(services, t)?.as_scalar()?;
            let max = inputs[2].clone()(services, t)?.as_scalar()?;
            Some(Vector::scalar(value.clamp(min, max)))
        }
        "stdlib::mix" => {
            if inputs.len() != 3 {
                return None;
            }
            let left = inputs[0].clone()(services, t)?.as_scalar()?;
            let right = inputs[1].clone()(services, t)?.as_scalar()?;
            let blend = inputs[2].clone()(services, t)?.as_scalar()?;
            Some(Vector::scalar(left * (1.0 - blend) + right * blend))
        }
        "stdlib::step" => {
            if inputs.len() != 2 {
                return None;
            }
            let edge = inputs[0].clone()(services, t)?.as_scalar()?;
            let value = inputs[1].clone()(services, t)?.as_scalar()?;
            Some(Vector::scalar(if value >= edge { 1.0 } else { 0.0 }))
        }
        _ => None,
    }
}
