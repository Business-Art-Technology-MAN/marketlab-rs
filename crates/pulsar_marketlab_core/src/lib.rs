//! Core MarketLab resources: OpenUSD financial schema and shared constants.

mod orchestration;
mod schema_defaults;
mod schema_sidecar;

/// Canonical financial schema layer (`FinancialAsset`, `OtlOperator`, `PortfolioIntegrator`).
pub const FINANCIAL_SCHEMA_USDA: &str = include_str!("../resources/usd/schema.usda");

pub use orchestration::{
    compile, compile_script, cross, macd, parse, sma, tokenize, BinOp, CompileError, ComputedAttributeStream,
    ExecutionNode, Expr, GraphCompileSpec, GraphCompileWire, GraphEngineError,
    MarketLabGraphEngine, SeriesClosure, SignalTransformFn, StageGraphPrim, StageGraphSnapshot,
    Token,
};
pub use schema_defaults::financial_schema_defaults;
pub use schema_sidecar::{
    embed_schema_inline_in_layer, ensure_schema_sidecar_for_document, initial_stage_usda,
    schema_class_definitions_usda, schema_sidecar_directory, schema_sidecar_path_for_document,
    schema_sidecar_usda, SCHEMA_SIDECAR_FILENAME, SCHEMA_SUBLAYER_REF,
};

#[cfg(test)]
mod tests {
    use super::FINANCIAL_SCHEMA_USDA;
    use super::{
        ensure_schema_sidecar_for_document, schema_sidecar_path_for_document,
    };

    #[test]
    fn schema_defines_financial_typed_classes() {
        for class in ["FinancialAsset", "OtlOperator", "PortfolioIntegrator"] {
            assert!(
                FINANCIAL_SCHEMA_USDA.contains(&format!("class \"{class}\"")),
                "missing typed class {class}"
            );
        }
        assert!(FINANCIAL_SCHEMA_USDA.contains("inputs:symbol"));
        assert!(FINANCIAL_SCHEMA_USDA.contains("inputs:script_src"));
        assert!(FINANCIAL_SCHEMA_USDA.contains("outputs:portfolio_wealth"));
    }

    #[test]
    fn initial_stage_embeds_financial_schema_classes() {
        let usda = super::initial_stage_usda();
        assert!(usda.contains("class \"FinancialAsset\""));
        assert!(usda.contains("class \"OtlOperator\""));
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
