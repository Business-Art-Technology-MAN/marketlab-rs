//! Unified OTL script resolution and normalization for canvas, USDA, and graph engine.

use super::compiler::{CompileError, CompiledSeries};
use crate::ta_uber_signal::{compose_uber_script_src, TaUberSignalConfig};

/// Inputs required to resolve the canonical `inputs:script_src` string for an OTL node.
#[derive(Clone, Debug, Default)]
pub struct OtlScriptContext<'a> {
    pub dsl_formula: Option<&'a str>,
    pub node_script: Option<&'a str>,
    /// Legacy indicator id (hydrate compat); ignored when `uber_config` is set.
    pub indicator_id: Option<&'a str>,
    pub lookback_period: u32,
    pub uber_config: Option<&'a TaUberSignalConfig>,
}

/// Resolve the authoritative OTL source string (DSL > node script > uber compose > legacy shorthand).
pub fn resolve_otl_script_src(ctx: &OtlScriptContext<'_>) -> String {
    if let Some(formula) = ctx
        .dsl_formula
        .map(str::trim)
        .filter(|text| !text.is_empty())
    {
        return formula.to_string();
    }
    if let Some(script) = ctx.node_script.map(str::trim).filter(|text| !text.is_empty()) {
        return script.to_string();
    }
    if let Some(config) = ctx.uber_config {
        return compose_uber_script_src(config);
    }
    let indicator_id = ctx.indicator_id.unwrap_or("rsi");
    format!("{indicator_id}(period={})", ctx.lookback_period)
}

/// Normalize playhead-oriented OTL syntax into vectorized series form (`data`-first TA calls).
pub fn normalize_for_series_eval(source: &str) -> String {
    let trimmed = source.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    let mapped_input = remap_input_identifiers(trimmed);

    if let Some(expanded) = expand_indicator_shorthand(&mapped_input) {
        return expanded;
    }

    normalize_unary_ta_calls(&mapped_input)
}

fn remap_input_identifiers(source: &str) -> String {
    source.replace("(input,", "(data,").replace("(input ", "(data ")
}

/// Parse and compile OTL using shared normalization (graph engine entry point).
pub fn compile_unified_script(source: &str) -> Result<CompiledSeries, CompileError> {
    use super::compiler::{
        compile_script_multi_with_context, normalize_script_for_compile, ScriptCompileContext,
    };

    let ctx = ScriptCompileContext::from_script_source(source);
    let stripped = normalize_script_for_compile(source);
    let normalized = normalize_for_series_eval(&stripped);
    compile_script_multi_with_context(&normalized, &ctx)
}

fn expand_indicator_shorthand(source: &str) -> Option<String> {
    let open = source.find('(')?;
    let name = source[..open].trim();
    if name.is_empty() || !name.chars().all(is_ta_name_char) {
        return None;
    }
    let close = source.rfind(')')?;
    let args = source[open + 1..close].trim();
    if args.is_empty() {
        return None;
    }

    let period = if let Some(rest) = args.strip_prefix("period=") {
        rest.trim().parse::<usize>().ok()?
    } else if args.chars().all(|ch| ch.is_ascii_digit()) {
        args.parse::<usize>().ok()?
    } else {
        return None;
    };

    if period == 0 {
        return None;
    }

    Some(format!("{name}(data, {period})"))
}

fn is_ta_name_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '_' | ':')
}

fn normalize_unary_ta_calls(source: &str) -> String {
    const UNARY_TA: &[&str] = &[
        "sma", "ema", "rsi", "wma", "stddev", "variance", "atr",
        "ta::sma", "ta::ema", "ta::rsi", "ta::wma", "ta::stddev", "ta::variance", "ta::atr",
    ];
    let mut out = source.to_string();
    for name in UNARY_TA {
        let needle = format!("{name}(");
        let mut search_from = 0;
        while let Some(rel) = out[search_from..].find(&needle) {
            let start = search_from + rel + needle.len();
            let rest = &out[start..];
            let Some(end) = find_call_arg_end(rest) else {
                break;
            };
            let arg = rest[..end].trim();
            if arg.is_empty() || arg.contains(',') || arg.contains('(') {
                search_from = start;
                continue;
            }
            if matches!(arg, "data" | "input" | "close" | "price" | "x") {
                search_from = start;
                continue;
            }
            if arg.chars().all(|ch| ch.is_ascii_digit() || ch == '.') {
                let replacement = format!("{name}(data, {arg})");
                let call_start = search_from + rel;
                let call_end = start + end + 1;
                out.replace_range(call_start..call_end, &replacement);
                search_from = call_start + replacement.len();
                continue;
            }
            search_from = start;
        }
    }
    out
}

fn find_call_arg_end(rest: &str) -> Option<usize> {
    let mut depth = 0usize;
    for (index, ch) in rest.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' if depth == 0 => return Some(index),
            ')' => depth -= 1,
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_prefers_dsl_formula() {
        let ctx = OtlScriptContext {
            dsl_formula: Some("close - sma(3)"),
            node_script: Some("identity"),
            indicator_id: Some("macd"),
            lookback_period: 14,
            uber_config: None,
        };
        assert_eq!(resolve_otl_script_src(&ctx), "close - sma(3)");
    }

    #[test]
    fn resolve_falls_back_to_indicator_shorthand() {
        let ctx = OtlScriptContext {
            dsl_formula: None,
            node_script: None,
            indicator_id: Some("rsi"),
            lookback_period: 21,
            uber_config: None,
        };
        assert_eq!(resolve_otl_script_src(&ctx), "rsi(period=21)");
    }

    #[test]
    fn normalize_expands_indicator_period_shorthand() {
        assert_eq!(
            normalize_for_series_eval("rsi(period=14)"),
            "rsi(data, 14)"
        );
    }

    #[test]
    fn normalize_expands_unary_sma() {
        assert_eq!(
            normalize_for_series_eval("cross(sma(3), sma(10))"),
            "cross(sma(data, 3), sma(data, 10))"
        );
    }

    #[test]
    fn compile_unified_accepts_playhead_style_formula() {
        let data = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
        let compiled = compile_unified_script("sma(3)").expect("compile");
        let CompiledSeries::Single(closure) = compiled else {
            panic!("expected single series");
        };
        let out = closure(&data);
        assert_eq!(out.len(), data.len());
        assert!(out[1].is_nan());
        assert_eq!(out[2], 2.0);
    }

    #[test]
    fn compile_unified_accepts_osl_shader_with_body_assignments() {
        let data = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
        let script = r#"
            float source,
            int lookback,
            output float signal
        {
            lookback = 3;
            signal = sma(source, 3);
        }"#;
        let compiled = compile_unified_script(script).expect("compile");
        let CompiledSeries::Single(closure) = compiled else {
            panic!("expected single series");
        };
        let out = closure(&data);
        assert_eq!(out[2], 2.0);
    }

    #[test]
    fn compile_unified_accepts_fn_main_with_body_assignments() {
        let data = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
        let script = "fn main(source, lookback) { lookback = 3; return sma(source, 3); }";
        let compiled = compile_unified_script(script).expect("compile");
        let CompiledSeries::Single(closure) = compiled else {
            panic!("expected single series");
        };
        let out = closure(&data);
        assert_eq!(out[2], 2.0);
    }

    #[test]
    fn resolve_uber_config_script() {
        use crate::ta_uber_signal::{TaArchetype, TaUberSignalConfig};
        let config = TaUberSignalConfig::new(TaArchetype::Trend);
        let ctx = OtlScriptContext {
            uber_config: Some(&config),
            ..Default::default()
        };
        assert_eq!(resolve_otl_script_src(&ctx), "ta::sma(input, 14)");
    }
}
