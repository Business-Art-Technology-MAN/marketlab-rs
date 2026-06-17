//! OTL three-tier object frontend (`signal`, `allocator`, `portfolio`).

mod codegen;
mod ast;
mod lexer;
mod parser;
mod validate;

pub use ast::{
    OtlObjectDeclaration, OtlObjectKind, OtlProgram, OtlType, PortDirection, PropertyDeclaration,
    Statement,
};
pub use codegen::{
    apply_alpha_conviction, conviction_scale_from_signal_series, resolve_runtime_script_source,
    ResolvedOtlSource,
};
pub use lexer::{object_kind_from_token, tokenize as tokenize_object_declarations};
pub use parser::{parse_program, ParseError};
pub use validate::{validate_object, validate_program, ValidationError};

/// Parse and semantically validate OTL object source.
pub fn compile_object_program(source: &str) -> Result<OtlProgram, FrontendError> {
    let program = parse_program(source)?;
    validate_program(&program)?;
    Ok(program)
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum FrontendError {
    #[error(transparent)]
    Parse(#[from] ParseError),
    #[error(transparent)]
    Validation(#[from] ValidationError),
}

#[cfg(test)]
mod tests {
    use super::*;

    const TREND_GATE: &str = r#"
signal trend_gate(
    input closure upstream,
    output closure gated
) {
    gated = upstream;
}
"#;

    const HRP_ALLOCATOR: &str = r#"
allocator hrp_blend(
    input closure[] legs,
    output closure blended
) {
    blended = mix(legs[0], legs[1], 0.5);
}
"#;

    const MASTER_PORTFOLIO: &str = r#"
portfolio master_book(
    input closure[] books,
    output closure execution_map
) {
    execution_map = portfolio_info("execution_map");
}
"#;

    #[test]
    fn parses_signal_object_declaration() {
        let program = parse_program(TREND_GATE).expect("parse signal");
        assert_eq!(program.objects.len(), 1);
        assert_eq!(program.objects[0].kind, OtlObjectKind::Signal);
        assert_eq!(program.objects[0].name, "trend_gate");
    }

    #[test]
    fn validates_signal_rejects_closure_array_inputs() {
        let invalid = r#"
signal bad(
    input closure[] legs,
    output closure gated
) {
    gated = legs[0];
}
"#;
        let program = parse_program(invalid).expect("parse");
        let err = validate_program(&program).expect_err("reject closure[] on signal");
        assert!(matches!(
            err,
            ValidationError::SignalClosureArrayInput { .. }
        ));
    }

    #[test]
    fn validates_allocator_rejects_portfolio_info() {
        let invalid = r#"
allocator bad(
    input closure leg,
    output closure blended
) {
    blended = portfolio_info("drawdown");
}
"#;
        let program = parse_program(invalid).expect("parse");
        let err = validate_program(&program).expect_err("reject portfolio_info in allocator");
        assert!(matches!(
            err,
            ValidationError::AllocatorPortfolioStateAccess { .. }
        ));
    }

    #[test]
    fn legacy_osl_desugars_to_legacy_shader_object() {
        let legacy = r#"
void adaptive_trigger(
    float source,
    int lookback,
    output float signal
) {
    signal = sma(source, lookback);
}
"#;
        let program = parse_program(legacy).expect("legacy desugar");
        assert_eq!(program.objects.len(), 1);
        assert_eq!(program.objects[0].kind, OtlObjectKind::LegacyShader);
    }

    #[test]
    fn compile_object_program_accepts_valid_allocator() {
        compile_object_program(HRP_ALLOCATOR).expect("valid allocator");
        compile_object_program(MASTER_PORTFOLIO).expect("valid portfolio");
    }
}
