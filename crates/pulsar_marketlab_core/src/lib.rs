//! Core MarketLab resources: OpenUSD financial schema and shared constants.

mod orchestration;
mod frontend;
mod schema_defaults;
mod schema_sidecar;
mod ta_uber_signal;

/// Canonical financial schema layer (`FinancialAsset`, `OtlOperator`, `PortfolioIntegrator`).
pub const FINANCIAL_SCHEMA_USDA: &str = include_str!("../resources/usd/schema.usda");

pub use orchestration::{
    compile, compile_script, compile_script_multi, compile_script_multi_with_context,
    deserialize_from_bytes, load_compiled_asset_from_path, manifest_json_from_signature,
    serialize_to_bytes, NodeManifest, OtcBinaryDecoder, OtcBinaryEncoder, OtcBinaryHeader,
    OtcCompiledAsset, OtcError, CURRENT_ENGINE_GENERATION,
    compile_unified_script, cross, display_name_for_script, ema, macd,
    normalize_for_series_eval, normalize_script_for_compile, parse, parse_script_entry_point_name,
    parse_script_scalar_uniforms, parse_script_signature, parse_with_context,
    resolve_otl_script_src, rsi, set_script_uniform_default, sma, tokenize, BinOp, CompileError,
    CompiledSeries, ComputedAttributeStream, EvaluationContext, ExecutionNode, Expr, GraphCompileSpec, OslParamType,
    OslParameter, ScriptCompileContext, ScriptSignature, GraphCompileWire, GraphEngineError,
    MultiSeriesClosure, OtlScriptContext, MarketLabGraphEngine, PortfolioIntegrationResult,
    PortfolioTrackingFrame, SeriesClosure, SignalTransformFn, StageGraphPrim, StageGraphSnapshot,
    SymbolicOtlClosure, TimelineExecutionResult, Token,
};
pub use ta_uber_signal::{
    algorithm_display_label, compose_uber_script_src, hyperparameter_visibility,
    infer_archetype_from_algorithm, node_display_name, TaArchetype, TaHyperparamVisibility,
    TaUberSignalConfig,
};
pub use schema_defaults::financial_schema_defaults;
pub use schema_sidecar::{
    embed_schema_inline_in_layer, ensure_schema_sidecar_for_document, initial_stage_usda,
    schema_class_definitions_usda, schema_sidecar_directory, schema_sidecar_path_for_document,
    schema_sidecar_usda, SCHEMA_SIDECAR_FILENAME, SCHEMA_SUBLAYER_REF,
};
pub use frontend::{
    compile_object_program, object_kind_from_token, parse_program as parse_object_program,
    resolve_runtime_script_source, ResolvedOtlSource,
    tokenize_object_declarations, validate_object, validate_program, FrontendError,
    OtlObjectDeclaration, OtlObjectKind, OtlProgram, OtlType, ParseError as OtlParseError,
    PortDirection, PropertyDeclaration, Statement, ValidationError,
};

#[cfg(test)]
mod tests {
    use super::FINANCIAL_SCHEMA_USDA;
    use super::{
        ensure_schema_sidecar_for_document, schema_sidecar_path_for_document,
    };

    #[test]
    fn schema_defines_financial_typed_classes() {
        for class in [
            "FinancialAsset",
            "OtlOperator",
            "OtlTaUberSignal",
            "PortfolioIntegrator",
        ] {
            assert!(
                FINANCIAL_SCHEMA_USDA.contains(&format!("class \"{class}\"")),
                "missing typed class {class}"
            );
        }
        assert!(FINANCIAL_SCHEMA_USDA.contains("inputs:symbol"));
        assert!(FINANCIAL_SCHEMA_USDA.contains("inputs:script_src"));
        assert!(FINANCIAL_SCHEMA_USDA.contains("inputs:script_compiled_path"));
        assert!(FINANCIAL_SCHEMA_USDA.contains("outputs:portfolio_wealth"));
    }

    #[test]
    fn initial_stage_embeds_financial_schema_classes() {
        let usda = super::initial_stage_usda();
        assert!(usda.contains("class \"FinancialAsset\""));
        assert!(usda.contains("class \"OtlOperator\""));
        assert!(usda.contains("class \"OtlTaUberSignal\""));
        assert!(usda.contains("info:archetype"));
        assert!(usda.contains("class \"PortfolioIntegrator\""));
        assert!(usda.contains("def Scope \"MarketLab\""));
        assert!(!usda.contains("subLayers"));
    }

    #[test]
    fn ensure_schema_sidecar_materializes_missing_file() {
        use std::fs;

        let dir = std::env::temp_dir().join(format!("marketlab_schema_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("temp dir");
        let doc = dir.join("stage.usda");
        fs::write(&doc, "placeholder").expect("write doc");

        ensure_schema_sidecar_for_document(&doc).expect("materialize sidecar");
        assert!(schema_sidecar_path_for_document(&doc).is_file());
        let _ = fs::remove_dir_all(&dir);
    }
}
