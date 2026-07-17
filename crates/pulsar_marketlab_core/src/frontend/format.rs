//! Pretty-printer for canonical OTL object declarations (three-tier + OSL shader).

use super::ast::{
    OtlObjectDeclaration, OtlProgram, OtlType, PortDirection, PropertyDeclaration, Statement,
};

/// Render an OTL program as spec-canonical source text.
pub fn format_program(program: &OtlProgram) -> String {
    program
        .objects
        .iter()
        .map(format_object)
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Render one `signal` / `allocator` / `portfolio` / `shader` declaration.
pub fn format_object(object: &OtlObjectDeclaration) -> String {
    let mut out = String::new();
    out.push_str(object.kind.keyword());
    out.push(' ');
    out.push_str(&object.name);
    out.push_str("(\n");

    let mut ports: Vec<&PropertyDeclaration> = object.inputs.iter().collect();
    ports.extend(object.outputs.iter());
    for (index, port) in ports.iter().enumerate() {
        let direction = match port.direction {
            PortDirection::Input => "input",
            PortDirection::Output => "output",
        };
        out.push_str("    ");
        out.push_str(direction);
        out.push(' ');
        out.push_str(format_type(port.ty));
        out.push(' ');
        out.push_str(&port.name);
        if port.direction == PortDirection::Input {
            if let Some(value) = port.default_value {
                out.push_str(" = ");
                out.push_str(&format_scalar_default(port.ty, value));
            }
        }
        if index + 1 < ports.len() {
            out.push(',');
        }
        out.push('\n');
    }

    out.push_str(") {\n");
    for statement in &object.body {
        format_statement(&mut out, statement);
    }
    out.push('}');
    out
}

fn format_scalar_default(ty: OtlType, value: f64) -> String {
    match ty {
        OtlType::Int => format!("{}", value.round() as i64),
        OtlType::Float => {
            if (value - value.round()).abs() <= f64::EPSILON {
                format!("{:.1}", value)
            } else {
                value.to_string()
            }
        }
        OtlType::String | OtlType::Closure | OtlType::ClosureArray => value.to_string(),
    }
}

fn format_type(ty: OtlType) -> &'static str {
    match ty {
        OtlType::Float => "float",
        OtlType::Int => "int",
        OtlType::String => "string",
        OtlType::Closure => "closure",
        OtlType::ClosureArray => "closure[]",
    }
}

fn format_statement(out: &mut String, statement: &Statement) {
    match statement {
        Statement::Assign { target, expr } => {
            out.push_str("    ");
            out.push_str(target);
            out.push_str(" = ");
            out.push_str(expr.trim().trim_end_matches(';'));
            out.push_str(";\n");
        }
        Statement::Return { expr } => {
            out.push_str("    return ");
            out.push_str(expr.trim().trim_end_matches(';'));
            out.push_str(";\n");
        }
        Statement::Raw { text } => {
            for line in text.lines() {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                out.push_str("    ");
                out.push_str(trimmed);
                if !trimmed.ends_with(';') && !trimmed.ends_with('}') {
                    out.push(';');
                }
                out.push('\n');
            }
        }
    }
}

/// Default starter script for new OTL editor tabs (OSL shader tier).
pub const OTL_DEFAULT_TEMPLATE: &str = r#"shader spread_sign(
    input float source,
    output float signal
) {
    signal = ta::spread_sign(ta::sma(source, 10), ta::sma(source, 50));
}
"#;

pub fn otl_default_template() -> &'static str {
    OTL_DEFAULT_TEMPLATE
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontend::ast::OtlObjectKind;

    #[test]
    fn formats_signal_object_with_closure_ports() {
        let object = OtlObjectDeclaration {
            kind: OtlObjectKind::Signal,
            name: "trend_gate".to_string(),
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
                expr: "raw".to_string(),
            }],
        };
        let formatted = format_object(&object);
        assert!(formatted.contains("signal trend_gate"));
        assert!(formatted.contains("input closure raw"));
        assert!(formatted.contains("output closure gated"));
        assert!(formatted.contains("gated = raw;"));
    }
}
