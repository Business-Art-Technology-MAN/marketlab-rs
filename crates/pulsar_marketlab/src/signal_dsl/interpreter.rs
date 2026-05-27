//! OTL closure compiler and runtime interpreter.

use std::sync::Arc;

use crate::technical_analysis::{compute_ta_latest_with_params, MarketSeriesWindow};

use super::ast::{DslError, DslExpression};
use super::parser::parse;
use super::services::{MarketProviderServices, OtlClosure};
use super::vector::Vector;

pub const DEFAULT_CLOSE_PATH: &str = "/assets/active/close";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompileContext {
    pub timeline_close_path: String,
}

impl Default for CompileContext {
    fn default() -> Self {
        Self {
            timeline_close_path: DEFAULT_CLOSE_PATH.to_string(),
        }
    }
}

pub fn compile(expr: &DslExpression, ctx: &CompileContext) -> Result<OtlClosure, DslError> {
    match expr {
        DslExpression::Literal(value) => {
            let scalar = f64::from(*value);
            Ok(Arc::new(move |_, _| Some(Vector::scalar(scalar))))
        }
        DslExpression::Variable(name) => {
            let name = name.clone();
            Ok(Arc::new(move |services, t| services.get_global_attribute(&name, t)))
        }
        DslExpression::BinaryOp(left, op, right) => {
            let left = compile(left, ctx)?;
            let right = compile(right, ctx)?;
            let op = *op;
            Ok(Arc::new(move |services, t| {
                let left = left(services, t)?.as_scalar()?;
                let right = right(services, t)?.as_scalar()?;
                let value = match op {
                    '+' => left + right,
                    '-' => left - right,
                    '*' => left * right,
                    '/' => {
                        if right.abs() <= f64::EPSILON {
                            return None;
                        }
                        left / right
                    }
                    _ => return None,
                };
                Some(Vector::scalar(value))
            }))
        }
        DslExpression::FunctionCall { name, args } => compile_function_call(name, args, ctx),
    }
}

pub fn compile_formula(formula: &str, ctx: &CompileContext) -> Result<OtlClosure, DslError> {
    compile(&parse(formula)?, ctx)
}

pub fn invoke_closure(
    closure: &OtlClosure,
    services: &dyn MarketProviderServices,
    t: f64,
) -> Result<f32, DslError> {
    closure(services, t)
        .and_then(|vector| vector.as_scalar())
        .map(|value| value as f32)
        .ok_or_else(|| DslError::Evaluation("closure returned no scalar vector".into()))
}

/// Backward-compatible helper for graph nodes backed by a rolling bar window.
pub fn evaluate_formula(
    formula: &str,
    window: &MarketSeriesWindow,
    lookback: usize,
) -> Result<f32, DslError> {
    let closure = compile_formula(formula, &CompileContext::default())?;
    let provider = WindowMarketProvider { window, lookback };
    let t = window.len().saturating_sub(1) as f64;
    invoke_closure(&closure, &provider, t)
}

fn compile_function_call(
    name: &str,
    args: &[DslExpression],
    ctx: &CompileContext,
) -> Result<OtlClosure, DslError> {
    if let Some(integrator) = name.strip_prefix("integrator::") {
        return compile_integrator_call(integrator, args, ctx);
    }

    match name {
        "sma" | "ta::sma" => compile_sma_intrinsic(args, ctx),
        "rsi" | "ta::rsi" => compile_integrator_call("vector_ta::rsi", args, ctx),
        "rtn::log" => compile_financial_intrinsic("financial::rtn_log", args, ctx),
        "vol::realized" => compile_financial_intrinsic("financial::vol_realized", args, ctx),
        "vol::parkinson" => compile_financial_intrinsic("financial::vol_parkinson", args, ctx),
        "clamp" => compile_integrator_call("stdlib::clamp", args, ctx),
        "mix" => compile_integrator_call("stdlib::mix", args, ctx),
        "step" => compile_integrator_call("stdlib::step", args, ctx),
        other => Err(DslError::UnknownFunction(other.to_string())),
    }
}

fn compile_financial_intrinsic(
    integrator_name: &str,
    args: &[DslExpression],
    ctx: &CompileContext,
) -> Result<OtlClosure, DslError> {
    if args.len() != 1 {
        return Err(DslError::InvalidArgumentCount {
            name: integrator_name.to_string(),
            expected: 1,
            got: args.len(),
        });
    }
    let period_closure = compile(&args[0], ctx)?;
    let close_path = ctx.timeline_close_path.clone();
    let integrator_name = integrator_name.to_string();
    Ok(Arc::new(move |services, t| {
        super::financial::execute_financial_integrator(
            services,
            &integrator_name,
            &close_path,
            std::slice::from_ref(&period_closure),
            t,
        )
    }))
}

fn compile_integrator_call(
    integrator_name: &str,
    args: &[DslExpression],
    ctx: &CompileContext,
) -> Result<OtlClosure, DslError> {
    let arg_closures = compile_args(args, ctx)?;
    let integrator_name = integrator_name.to_string();
    Ok(Arc::new(move |services, t| {
        services.execute_integrator(&integrator_name, &arg_closures, t)
    }))
}

fn compile_sma_intrinsic(args: &[DslExpression], ctx: &CompileContext) -> Result<OtlClosure, DslError> {
    if args.len() != 1 {
        return Err(DslError::InvalidArgumentCount {
            name: "sma".into(),
            expected: 1,
            got: args.len(),
        });
    }
    let period_closure = compile(&args[0], ctx)?;
    let path = ctx.timeline_close_path.clone();
    Ok(Arc::new(move |services, t| {
        execute_sma_intrinsic(services, &path, &period_closure, t)
    }))
}

fn compile_args(args: &[DslExpression], ctx: &CompileContext) -> Result<Vec<OtlClosure>, DslError> {
    args.iter().map(|arg| compile(arg, ctx)).collect()
}

fn execute_sma_intrinsic(
    services: &dyn MarketProviderServices,
    path: &str,
    period_closure: &OtlClosure,
    t: f64,
) -> Option<Vector> {
    let period = period_closure(services, t)?.as_scalar()?.round().max(1.0);
    let start = t - period + 1.0;
    let samples = services.sample_timeline(path, start, t);
    if samples.is_empty() {
        return None;
    }
    let sum: f64 = samples.iter().filter_map(|sample| sample.as_scalar()).sum();
    Some(Vector::scalar(sum / samples.len() as f64))
}

struct WindowMarketProvider<'a> {
    window: &'a MarketSeriesWindow,
    lookback: usize,
}

impl MarketProviderServices for WindowMarketProvider<'_> {
    fn sample_timeline(&self, path: &str, start_time: f64, end_time: f64) -> Vec<Vector> {
        if self.window.is_empty() || !path.ends_with("/close") {
            return Vec::new();
        }
        let start = start_time.max(0.0).floor() as usize;
        let end = end_time.min(self.window.len() as f64 - 1.0).floor() as usize;
        if start > end {
            return Vec::new();
        }
        self.window.close[start..=end]
            .iter()
            .map(|value| Vector::scalar(*value))
            .collect()
    }

    fn get_global_attribute(&self, name: &str, t: f64) -> Option<Vector> {
        if self.window.is_empty() {
            return None;
        }
        let index = (t.round() as usize).min(self.window.len() - 1);
        let value = match name {
            "close" => self.window.close[index],
            "open" => self.window.open[index],
            "high" => self.window.high[index],
            "low" => self.window.low[index],
            "volume" => self.window.volume[index],
            _ => return None,
        };
        if value.is_finite() {
            Some(Vector::scalar(value))
        } else {
            None
        }
    }

    fn execute_integrator(
        &self,
        integrator_name: &str,
        inputs: &[OtlClosure],
        t: f64,
    ) -> Option<Vector> {
        if let Some(value) =
            super::financial::execute_stdlib_integrator(self, integrator_name, inputs, t)
        {
            return Some(value);
        }
        if integrator_name.starts_with("financial::") {
            return super::financial::execute_financial_integrator(
                self,
                integrator_name,
                DEFAULT_CLOSE_PATH,
                inputs,
                t,
            );
        }
        match integrator_name {
            "vector_ta::rsi" => {
                let period = inputs
                    .first()?
                    .clone()(self, t)?
                    .as_scalar()?
                    .round()
                    .max(1.0) as usize;
                compute_ta_latest_with_params("rsi", self.window, period)
                    .map(Vector::scalar)
            }
            other => {
                let _ = (other, t, self.lookback);
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signal_dsl::mock_provider::MockMarketProvider;

    fn sample_provider() -> MockMarketProvider {
        MockMarketProvider::with_price_path(
            DEFAULT_CLOSE_PATH,
            &[
                (0.0, 100.0),
                (1.0, 102.0),
                (2.0, 101.0),
                (3.0, 105.0),
                (4.0, 104.0),
            ],
        )
    }

    #[test]
    fn compile_literal_returns_constant_closure() {
        let closure = compile(&DslExpression::Literal(7.5), &CompileContext::default()).unwrap();
        let provider = sample_provider();
        assert_eq!(
            invoke_closure(&closure, &provider, 4.0).unwrap(),
            7.5
        );
    }

    #[test]
    fn compile_arithmetic_closure_at_playhead() {
        let expr = parse("1.5 + 2 * 3").unwrap();
        let closure = compile(&expr, &CompileContext::default()).unwrap();
        let provider = sample_provider();
        assert!((invoke_closure(&closure, &provider, 4.0).unwrap() - 7.5).abs() < f32::EPSILON);
    }

    #[test]
    fn compile_close_minus_sma_matches_baseline() {
        let expr = parse("close - sma(3)").unwrap();
        let closure = compile(&expr, &CompileContext::default()).unwrap();
        let provider = sample_provider();
        let value = invoke_closure(&closure, &provider, 4.0).unwrap();
        assert!((value - (104.0 - 103.333_333)).abs() < 0.01);
    }

    #[test]
    fn compile_rsi_via_integrator_delegation() {
        let expr = parse("rsi(3)").unwrap();
        let closure = compile(&expr, &CompileContext::default()).unwrap();
        let provider = sample_provider();
        let value = invoke_closure(&closure, &provider, 4.0).unwrap();
        assert!(value.is_finite());
    }

    #[test]
    fn compile_integrator_explicit_form() {
        let expr = parse("integrator::sum(close, close)").unwrap();
        let closure = compile(&expr, &CompileContext::default()).unwrap();
        let provider = sample_provider();
        let value = invoke_closure(&closure, &provider, 4.0).unwrap();
        assert!((value - 208.0).abs() < f32::EPSILON);
    }

    #[test]
    fn evaluate_formula_window_adapter_still_works() {
        let mut window = MarketSeriesWindow::default();
        for close in [100.0, 102.0, 101.0, 105.0, 104.0] {
            window.push_close_only(close);
        }
        let value = evaluate_formula("close - sma(3)", &window, 14).unwrap();
        assert!((value - (104.0 - 103.333_333)).abs() < 0.01);
    }

    #[test]
    fn rtn_log_returns_historical_log_differential() {
        let closure = compile_formula("rtn::log(3)", &CompileContext::default()).unwrap();
        let provider = sample_provider();
        let value = invoke_closure(&closure, &provider, 4.0).unwrap();
        let expected = (104.0_f64 / 102.0).ln() as f32;
        assert!((value - expected).abs() < 0.001, "got {value}, expected {expected}");
    }

    #[test]
    fn vol_realized_returns_sample_standard_deviation() {
        let closure = compile_formula("vol::realized(3)", &CompileContext::default()).unwrap();
        let provider = sample_provider();
        let value = invoke_closure(&closure, &provider, 4.0).unwrap();
        assert!(value.is_finite());
        assert!(value > 0.0);
    }

    #[test]
    fn clamp_limits_financial_intrinsic_output() {
        let closure =
            compile_formula("clamp(rtn::log(3), -0.05, 0.05)", &CompileContext::default()).unwrap();
        let provider = sample_provider();
        let value = invoke_closure(&closure, &provider, 4.0).unwrap();
        assert!(value >= -0.05);
        assert!(value <= 0.05);
    }

    #[test]
    fn mix_blends_close_sma_and_realized_volatility() {
        let closure = compile_formula(
            "mix(close, sma(3), vol::realized(3))",
            &CompileContext::default(),
        )
        .unwrap();
        let provider = sample_provider();
        let value = invoke_closure(&closure, &provider, 4.0).unwrap();
        assert!(value.is_finite());
    }

    #[test]
    fn financial_closures_are_send_and_sync() {
        let closure = compile_formula("rtn::log(3)", &CompileContext::default()).unwrap();
        fn assert_send_sync<T: Send + Sync>(_: &T) {}
        assert_send_sync(&closure);
    }
}
