//! OTL three-tier object frontend (`signal`, `allocator`, `portfolio`).

mod ast;
mod canonical;
mod codegen;
mod format;
mod lexer;
mod parser;
mod validate;

pub use ast::{
    OtlObjectDeclaration, OtlObjectKind, OtlProgram, OtlType, PortDirection, PropertyDeclaration,
    Statement,
};
pub use canonical::{
    canonicalize_otl_source, signal_object_from_expression, shader_object_from_osl_source,
};
pub(crate) use canonical::shader_uses_osl_parameter_syntax;
pub use codegen::{
    apply_alpha_conviction, conviction_scale_from_signal_series, expression_from_object,
    resolve_runtime_script_source, series_expression_from_source, signal_expression_from_object,
    ResolvedOtlSource,
};
pub use format::{format_object, format_program, otl_default_template, OTL_DEFAULT_TEMPLATE};
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
    fn legacy_osl_desugars_to_shader_object() {
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
        assert_eq!(program.objects[0].name, "adaptive_trigger");
    }

    #[test]
    fn bare_expression_desugars_to_signal_object() {
        let program = parse_program("sma(input, 14)").expect("parse expression");
        assert_eq!(program.objects[0].kind, OtlObjectKind::Signal);
    }

    #[test]
    fn canonicalize_roundtrip_preserves_compile_surface() {
        let source = "ta::sma(input, 10)";
        let canonical = canonicalize_otl_source(source).expect("canonicalize");
        assert!(canonical.contains("signal "));
        assert!(canonical.contains("gated = ta::sma(input, 10);"));
    }

    #[test]
    fn parses_osl_shader_ma_crossover_for_tier_compile() {
        let src = r#"shader ma_crossover(
    float source,
    int fast = 10,
    int slow = 50,
    output float signal
) {
    signal = ta::cross(ta::sma(source, fast), ta::sma(source, slow));
}"#;
        let program = parse_program(src).expect("parse OSL shader");
        assert_eq!(program.objects.len(), 1);
        assert_eq!(program.objects[0].kind, OtlObjectKind::LegacyShader);
        canonicalize_otl_source(src).expect("canonicalize ma_crossover");
        compile_object_program(src).expect("compile_object_program ma_crossover");
    }

    #[test]
    fn canonicalize_preserves_scalar_uniform_defaults() {
        let src = r#"shader ma_crossover(
    float source,
    int fast = 10,
    int slow = 50,
    output float signal
) {
    signal = ta::cross(ta::sma(source, fast), ta::sma(source, slow));
}"#;
        let canonical = canonicalize_otl_source(src).expect("canonicalize");
        assert!(
            canonical.contains("fast = 10"),
            "expected fast default in canonical script: {canonical}"
        );
        assert!(
            canonical.contains("slow = 50"),
            "expected slow default in canonical script: {canonical}"
        );
        let uniforms = crate::parse_script_scalar_uniforms(&canonical);
        assert_eq!(
            uniforms
                .iter()
                .find(|param| param.name == "fast")
                .and_then(|param| param.default_value),
            Some(10.0)
        );
        assert_eq!(
            uniforms
                .iter()
                .find(|param| param.name == "slow")
                .and_then(|param| param.default_value),
            Some(50.0)
        );
    }

    #[test]
    fn parses_sma_smoke_shader_declaration() {
        let src = r#"shader sma_smoke(
    input float source,
    output float signal
) {
    signal = ta::sma(source, 3);
}"#;
        canonicalize_otl_source(src).expect("canonicalize sma_smoke");
    }

    #[test]
    fn compile_object_program_accepts_valid_allocator() {
        compile_object_program(HRP_ALLOCATOR).expect("valid allocator");
        compile_object_program(MASTER_PORTFOLIO).expect("valid portfolio");
    }
}
