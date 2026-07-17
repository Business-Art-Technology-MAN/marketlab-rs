//! Ingest legacy OTL forms and emit spec-canonical source text.

use crate::{display_name_for_script, merge_script_scalar_uniforms, normalize_script_for_compile, parse_script_signature, OslParamType};

use super::ast::{
    OtlObjectDeclaration, OtlObjectKind, OtlType, PortDirection, PropertyDeclaration, Statement,
};
use super::format::{format_program, otl_default_template};
use super::parser::parse_program;
use super::{validate_program, FrontendError};

/// Normalize any supported OTL surface syntax into spec-canonical source text.
///
/// Execution paths should continue to accept both canonical and legacy forms; this is the
/// authoring/storage representation aligned with the three-tier + OSL shader grammar.
pub fn canonicalize_otl_source(source: &str) -> Result<String, FrontendError> {
    let stripped = strip_otl_line_comments(source);
    let trimmed = stripped.trim();
    if trimmed.is_empty() {
        return Ok(otl_default_template().to_string());
    }
    let program = parse_program(trimmed)?;
    validate_program(&program)?;
    let mut canonical = format_program(&program);
    canonical = merge_script_scalar_uniforms(trimmed, &canonical);
    Ok(canonical)
}

fn strip_otl_line_comments(source: &str) -> String {
    source
        .lines()
        .map(|line| line.split("//").next().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Build a canonical tier-1 `signal` wrapper for a bare series expression.
pub fn signal_object_from_expression(expr: &str) -> OtlObjectDeclaration {
    let trimmed = expr.trim().trim_end_matches(';');
    OtlObjectDeclaration {
        kind: OtlObjectKind::Signal,
        name: sanitize_object_name(&display_name_for_script(trimmed, "auto_series")),
        inputs: vec![PropertyDeclaration {
            direction: PortDirection::Input,
            ty: OtlType::Closure,
            name: "raw".to_string(),
            default_value: None,
        }],
        outputs: vec![PropertyDeclaration {
            direction: PortDirection::Output,
            ty: OtlType::Closure,
            name: "gated".to_string(),
            default_value: None,
        }],
        body: vec![Statement::Assign {
            target: "gated".to_string(),
            expr: trimmed.to_string(),
        }],
    }
}

/// Build a canonical OSL `shader` object from a typed header + braced body script.
pub fn shader_object_from_osl_source(source: &str) -> OtlObjectDeclaration {
    let signature = parse_script_signature(source);
    let name = crate::parse_script_entry_point_name(source)
        .map(|name| sanitize_object_name(&name))
        .unwrap_or_else(|| sanitize_object_name(&display_name_for_script(source, "shader")));

    let mut inputs = Vec::new();
    let mut outputs = Vec::new();
    if signature.parameters.is_empty() {
        for (index, input) in signature.inputs.iter().enumerate() {
            inputs.push(PropertyDeclaration {
                direction: PortDirection::Input,
                ty: if index == 0 {
                    OtlType::Float
                } else {
                    OtlType::Int
                },
                name: input.clone(),
                default_value: None,
            });
        }
        for output in &signature.outputs {
            outputs.push(PropertyDeclaration {
                direction: PortDirection::Output,
                ty: OtlType::Float,
                name: output.clone(),
                default_value: None,
            });
        }
    } else {
        let mut primary_series_registered = false;
        for param in &signature.parameters {
            let ty = match param.ty {
                OslParamType::Float => OtlType::Float,
                OslParamType::Int => OtlType::Int,
                OslParamType::String => OtlType::String,
            };
            let default_value = if param.is_output {
                None
            } else {
                match param.ty {
                    OslParamType::Float if !primary_series_registered => {
                        primary_series_registered = true;
                        None
                    }
                    OslParamType::Float | OslParamType::Int => param.default_value,
                    OslParamType::String => None,
                }
            };
            let port = PropertyDeclaration {
                direction: if param.is_output {
                    PortDirection::Output
                } else {
                    PortDirection::Input
                },
                ty,
                name: param.name.clone(),
                default_value,
            };
            if param.is_output {
                outputs.push(port);
            } else {
                inputs.push(port);
            }
        }
    }

    OtlObjectDeclaration {
        kind: OtlObjectKind::LegacyShader,
        name,
        inputs,
        outputs,
        body: osl_body_to_statements(source, &signature),
    }
}

pub(crate) fn ingest_unwrapped_source(source: &str) -> OtlObjectDeclaration {
    let trimmed = source.trim();
    if is_bare_expression(trimmed) {
        return signal_object_from_expression(trimmed);
    }
    if is_osl_block_source(trimmed) {
        return shader_object_from_osl_source(trimmed);
    }
    if trimmed.to_ascii_lowercase().contains("fn main") {
        return shader_object_from_osl_source(trimmed);
    }
    shader_object_from_osl_source(trimmed)
}

pub(crate) fn is_bare_expression(source: &str) -> bool {
    let trimmed = source.trim();
    !trimmed.is_empty()
        && !trimmed.contains('{')
        && !trimmed.to_ascii_lowercase().contains("fn main")
        && !trimmed.to_ascii_lowercase().starts_with("void ")
        && !starts_with_object_keyword(trimmed)
}

pub(crate) fn is_osl_block_source(source: &str) -> bool {
    let trimmed = source.trim();
    if !trimmed.contains('{') {
        return false;
    }
    let signature = parse_script_signature(trimmed);
    if !signature.parameters.is_empty() || !signature.outputs.is_empty() {
        return true;
    }
    trimmed
        .find('{')
        .map(|open| looks_like_osl_header(trimmed[..open].trim()))
        .unwrap_or(false)
}

fn looks_like_osl_header(header: &str) -> bool {
    let lower = header.to_ascii_lowercase();
    lower.contains("float")
        || lower.contains("int")
        || lower.contains("output")
        || lower.contains("string")
}

fn starts_with_object_keyword(source: &str) -> bool {
    matches!(
        source.split_whitespace().next(),
        Some("signal") | Some("allocator") | Some("portfolio") | Some("shader")
    )
}

/// True for `shader name(float source, ...)` OSL headers (not `shader name(input float source, ...)`).
pub(crate) fn shader_uses_osl_parameter_syntax(source: &str) -> bool {
    let lower = source.to_ascii_lowercase();
    let Some(shader_idx) = lower.find("shader") else {
        return false;
    };
    let after_shader = &source[shader_idx..];
    let Some(lparen) = after_shader.find('(') else {
        return false;
    };
    let params = after_shader[lparen + 1..].trim_start();
    let Some(head) = params
        .split(|ch: char| ch == ',' || ch == ')')
        .next()
        .map(str::trim)
    else {
        return false;
    };
    let head = head
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    head == "float"
        || head == "int"
        || head == "string"
        || head == "output"
}

fn osl_body_to_statements(source: &str, signature: &crate::ScriptSignature) -> Vec<Statement> {
    if let Some(body) = extract_braced_body(source) {
        let mut statements = parse_body_assignments(&body);
        if statements.is_empty() {
            let expr = normalize_script_for_compile(source).trim().to_string();
            if !expr.is_empty() {
                if let Some(output) = signature.outputs.first() {
                    statements.push(Statement::Assign {
                        target: output.clone(),
                        expr,
                    });
                } else {
                    statements.push(Statement::Return { expr });
                }
            }
        }
        if !statements.is_empty() {
            return statements;
        }
        return vec![Statement::Raw { text: body }];
    }

    vec![Statement::Raw {
        text: source.to_string(),
    }]
}

pub(crate) fn extract_braced_body(source: &str) -> Option<String> {
    let open = source.find('{')?;
    let rest = &source[open + 1..];
    let mut depth = 1usize;
    for (index, ch) in rest.char_indices() {
        match ch {
            '{' => depth += 1,
            '}' if depth == 1 => return Some(rest[..index].trim().to_string()),
            '}' => depth -= 1,
            _ => {}
        }
    }
    None
}

pub(crate) fn parse_body_assignments(body: &str) -> Vec<Statement> {
    let mut statements = Vec::new();
    for line in body.lines() {
        let line = line.split("//").next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("return ") {
            statements.push(Statement::Return {
                expr: rest.trim_end_matches(';').trim().to_string(),
            });
            continue;
        }
        if let Some((target, expr)) = line.split_once('=') {
            statements.push(Statement::Assign {
                target: strip_osl_assignment_target(target.trim()),
                expr: expr.trim_end_matches(';').trim().to_string(),
            });
        }
    }
    statements
}

/// Strip `float` / `int` / `string` prefixes from OSL local declarations (`float x = ...`).
pub(crate) fn strip_osl_assignment_target(target: &str) -> String {
    let mut parts = target.split_whitespace();
    match parts.next() {
        Some("float" | "int" | "string") => parts
            .next()
            .unwrap_or(target)
            .trim()
            .to_string(),
        _ => target.trim().to_string(),
    }
}

fn sanitize_object_name(name: &str) -> String {
    let mut out = String::new();
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            out.push(if out.is_empty() && ch.is_ascii_digit() {
                '_'
            } else {
                ch
            });
        } else if ch.is_whitespace() || ch == '-' {
            if !out.ends_with('_') && !out.is_empty() {
                out.push('_');
            }
        }
    }
    if out.is_empty() {
        "auto_series".to_string()
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontend::ast::OtlObjectKind;

    #[test]
    fn bare_expression_canonicalizes_to_signal_block() {
        let canonical =
            canonicalize_otl_source("ta::spread_sign(ta::sma(input, 10), ta::sma(input, 50))")
                .expect("canonicalize");
        assert!(canonical.contains("signal "));
        assert!(canonical.contains("input closure raw"));
        assert!(canonical.contains("output closure gated"));
        assert!(canonical.contains("gated = ta::spread_sign"));
    }

    #[test]
    fn osl_shader_canonicalizes_to_shader_block() {
        let source = r#"
float wedge,
output float volume
{
    volume = ga::wedge_volume(60);
}
"#;
        let canonical = canonicalize_otl_source(source).expect("canonicalize");
        assert!(canonical.contains("shader "));
        assert!(canonical.contains("input float wedge"));
        assert!(canonical.contains("output float volume"));
        assert!(canonical.contains("volume = ga::wedge_volume(60);"));
    }

    #[test]
    fn three_tier_object_is_idempotent() {
        let source = r#"
signal trend_gate(
    input closure raw,
    output closure gated
) {
    gated = raw;
}
"#;
        let once = canonicalize_otl_source(source).expect("once");
        let twice = canonicalize_otl_source(&once).expect("twice");
        assert_eq!(once, twice);
    }

    #[test]
    fn ingest_bare_expression_builds_signal_kind() {
        let object = ingest_unwrapped_source("sma(input, 14)");
        assert_eq!(object.kind, OtlObjectKind::Signal);
        assert_eq!(object.outputs[0].name, "gated");
    }
}
