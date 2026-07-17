//! Runtime script resolution for OTL three-tier object declarations.

use super::ast::{OtlObjectDeclaration, OtlObjectKind, Statement};
use super::{compile_object_program, FrontendError};

/// Resolved executable script text after object-declaration desugaring.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedOtlSource {
    pub kind: OtlObjectKind,
    pub runtime_script: String,
    pub object_name: String,
}

/// Parse OTL object syntax when present; otherwise treat source as legacy script.
pub fn resolve_runtime_script_source(source: &str) -> Result<ResolvedOtlSource, FrontendError> {
    let trimmed = source.trim();
    if trimmed.is_empty() {
        return Ok(ResolvedOtlSource {
            kind: OtlObjectKind::LegacyShader,
            runtime_script: String::new(),
            object_name: "legacy".to_string(),
        });
    }

    if starts_with_object_keyword(trimmed) {
        let program = compile_object_program(trimmed)?;
        let object = program
            .objects
            .first()
            .ok_or_else(|| FrontendError::Parse(super::ParseError::ExpectedObjectKind))?;
        return Ok(ResolvedOtlSource {
            kind: object.kind,
            runtime_script: body_to_runtime_script(object),
            object_name: object.name.clone(),
        });
    }

    let program = super::parse_program(trimmed)?;
    let object = program
        .objects
        .first()
        .ok_or_else(|| FrontendError::Parse(super::ParseError::ExpectedObjectKind))?;
    Ok(ResolvedOtlSource {
        kind: object.kind,
        runtime_script: body_to_runtime_script(object),
        object_name: object.name.clone(),
    })
}

fn starts_with_object_keyword(source: &str) -> bool {
    matches!(
        source.split_whitespace().next(),
        Some("signal") | Some("allocator") | Some("portfolio") | Some("shader")
    )
}

fn body_to_runtime_script(object: &OtlObjectDeclaration) -> String {
    object
        .body
        .iter()
        .filter_map(|statement| match statement {
            Statement::Assign { target, expr } => Some(format!("{target} = {expr}")),
            Statement::Return { expr } => Some(expr.clone()),
            Statement::Raw { text } => Some(text.clone()),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Extract the series math expression from a three-tier / shader OTL declaration.
pub fn series_expression_from_source(source: &str) -> Option<String> {
    let trimmed = source.trim();
    if !starts_with_object_keyword(trimmed) {
        return None;
    }
    let program = super::parse_program(trimmed).ok()?;
    let object = program.primary_object()?;
    Some(expression_from_object(object))
}

pub fn expression_from_object(object: &OtlObjectDeclaration) -> String {
    signal_expression_from_object(object)
}

/// Extract the series math for the primary output port (typically `signal`).
pub fn signal_expression_from_object(object: &OtlObjectDeclaration) -> String {
    let output_names: Vec<String> = object
        .outputs
        .iter()
        .map(|port| port.name.to_ascii_lowercase())
        .collect();

    let mut last_output_expr = None;
    for statement in &object.body {
        if let Statement::Assign { target, expr } = statement {
            let target = target.trim().to_ascii_lowercase();
            if output_names.iter().any(|name| name == &target) {
                last_output_expr = Some(expr.trim().trim_end_matches(';').trim().to_string());
            }
        }
    }
    if let Some(expr) = last_output_expr.filter(|expr| !expr.is_empty()) {
        return expr;
    }

    for statement in &object.body {
        match statement {
            Statement::Return { expr } => {
                return expr.trim().trim_end_matches(';').trim().to_string();
            }
            Statement::Assign { expr, .. } => {
                return expr.trim().trim_end_matches(';').trim().to_string();
            }
            Statement::Raw { text } if !text.trim().is_empty() => {
                let normalized = crate::normalize_script_for_compile(text);
                if !normalized.trim().is_empty() {
                    return normalized;
                }
            }
            _ => {}
        }
    }
    String::new()
}

/// Scale unitless closure conviction from signal magnitude (0.1–1.0).
pub fn conviction_scale_from_signal_series(series: &[f64]) -> f64 {
    if series.is_empty() {
        return 1.0;
    }
    let peak = series
        .iter()
        .map(|value| value.abs())
        .fold(0.0_f64, f64::max);
    if peak <= f64::EPSILON {
        return 1.0;
    }
    (peak / (peak + 1.0)).clamp(0.1, 1.0)
}

/// Apply optional alpha scaling hints from OTL object body text.
pub fn apply_alpha_conviction(raw_weight: f64, runtime_script: &str, signal_scale: f64) -> f64 {
    let mut weight = raw_weight * signal_scale;
    if runtime_script.contains("drawdown") {
        weight *= 0.85;
    }
    if runtime_script.contains("half") || runtime_script.contains("0.5") {
        weight *= 0.5;
    }
    weight.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desugars_signal_body_to_runtime_script() {
        let source = r#"
signal trend_gate(input closure raw, output closure gated) {
    gated = raw;
}
"#;
        let resolved = resolve_runtime_script_source(source).expect("parse signal");
        assert_eq!(resolved.kind, OtlObjectKind::Signal);
        assert!(resolved.runtime_script.contains("gated = raw"));
    }

    #[test]
    fn legacy_script_passthrough() {
        let resolved = resolve_runtime_script_source("sma(input, 14)").expect("legacy");
        assert_eq!(resolved.kind, OtlObjectKind::Signal);
        assert!(resolved.runtime_script.contains("gated = sma(input, 14)"));
    }
}
