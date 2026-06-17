//! Core MarketLab resources: OpenUSD financial schema and shared constants.

mod compiler;
mod engine;
mod execution_matrix;
mod finance_database_ingest;
mod frontend;
mod orchestration;
mod schema_defaults;
mod layer_stack;
mod sublayer_stack;
mod schema_sidecar;
mod snapshot_serialization;
mod ta_uber_signal;
mod taxonomy;

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
    wrap_series_script_as_signal_source,
    CompiledSeries, ComputedAttributeStream, ComputedTokenStream, EvaluationContext, ExecutionNode,
    Expr, GraphCompileSpec, OslParamType,
    OslParameter, ScriptCompileContext, ScriptSignature, GraphCompileWire, GraphEngineError,
    MultiSeriesClosure, OtlScriptContext, MarketLabGraphEngine, PortfolioIntegrationResult,
    ComposedAssetMeta, PathBindingIndex, PortfolioTrackingFrame, SeriesClosure, SignalTransformFn,
    StageGraphPrim, StageGraphSnapshot,
    SymbolicOtlClosure, TimelineExecutionResult, Token, AssetQuote, ClosureLegKind,
    DirectionalDistribution, compute_allocation_weights,
};
pub use snapshot_serialization::{
    deserialize_portfolio_weights, per_bar_weight_encodings, serialize_portfolio_weights,
    serialize_portfolio_weights_from_slices,
};
pub use ta_uber_signal::{
    algorithm_display_label, compose_uber_script_src, hyperparameter_visibility,
    infer_archetype_from_algorithm, node_display_name, TaArchetype, TaHyperparamVisibility,
    TaUberSignalConfig,
};
pub use schema_defaults::financial_schema_defaults;
pub use schema_sidecar::{
    embed_schema_inline_in_layer, ensure_schema_sidecar_for_document,
    ensure_metadata_library_sidecar_for_document, initial_stage_usda,
    metadata_sidecar_path_for_document, schema_class_definitions_usda, schema_sidecar_directory,
    schema_sidecar_path_for_document, schema_sidecar_usda, METADATA_LIBRARY_SIDECAR_FILENAME,
    METADATA_SUBLAYER_REF, SCHEMA_SIDECAR_FILENAME, SCHEMA_SUBLAYER_REF,
};
pub use sublayer_stack::{
    create_and_insert_sublayer, ensure_workstation_sublayer_stack,
    import_external_portfolio_layer, insert_sublayer_at, layer_path_for_root,
    parse_ordered_sublayer_filenames, prepend_sublayer, remove_sublayer, reorder_sublayer,
    sublayer_ref_to_filename, write_ordered_sublayers,
};
pub use layer_stack::{
    blank_project_session_layer_usda, finance_database_equities_empty_layer_usda,
    imported_portfolio_layer_filename, portfolio_import_insert_index, prim_display_label,
    session_layer_usda, signals_layer_usda, sp500_universe_layer_usda,
    workstation_root_layer_header, workstation_root_layer_header_with_stack,
    FINANCE_DATABASE_EQUITIES_LAYER_FILENAME, FINANCE_DATABASE_EQUITIES_SUBLAYER_REF,
    PORTFOLIOS_SCOPE, SESSION_LAYER_FILENAME, SESSION_SUBLAYER_REF, SIGNALS_LAYER_FILENAME,
    SIGNALS_SCOPE, SIGNALS_SUBLAYER_REF, SP500_SUBLAYER_REF, SP500_UNIVERSE_LAYER_FILENAME,
    UNIVERSE_SCOPE, USER_LABEL_ATTR, WORKSTATION_LAYER_STACK,
};
pub use finance_database_ingest::{
    exchange_token_to_mic, ingest_equities_csv, map_sector_to_inputs, sanitize_ticker_segment,
    stable_asset_prim_leaf, EquityCatalogRow, IngestError,
};
pub use taxonomy::{
    flatten_asset_metadata, FlattenedAssetMetadata, METADATA_LIBRARY_USDA,
};
pub use compiler::{
    compile_object_program as compile_object_tier, AllocatorExecutionEngine, CompiledProgramTier,
    ObjectCodegenRegistry, PortfolioExecutionEngine, SignalExecutionEngine,
};
pub use engine::{evaluate_compiled_tier, evaluate_node_vector_series};
pub use execution_matrix::{ColumnMajorBlock, ExecutionContext, GraphSeriesMatrix, RuntimeEngineError};
pub use arrow_array::types::Float64Type;
pub use arrow_array::{Float64Array, PrimitiveArray};
pub use engine::{
    allocation_weights_from_covariance, chronological_stride, compute_execution_levels,
    fill_subcovariance_block, format_timeline_tick, is_parallel_tier_signal,
    merge_parallel_signal_outcomes, run_parallel_signal_batch, shared_columns_from_vec,
    uses_covariance_optimizer, HistoricalTimelineMap, MarketTimelineWindow, ParallelSignalJob,
    ParallelSignalOutcome, ParallelSweepContext, PrecomputedMatrixCache, RollingMatrixWindow,
    SharedPriceColumn, DEFAULT_COVARIANCE_LOOKBACK,
};
pub use frontend::{
    apply_alpha_conviction, compile_object_program, conviction_scale_from_signal_series,
    object_kind_from_token, parse_program as parse_object_program, resolve_runtime_script_source,
    ResolvedOtlSource,
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
        assert!(FINANCIAL_SCHEMA_USDA.contains("info:sector"));
        assert!(FINANCIAL_SCHEMA_USDA.contains("info:cusip"));
        assert!(FINANCIAL_SCHEMA_USDA.contains("inputs:script_src"));
        assert!(FINANCIAL_SCHEMA_USDA.contains("inputs:script_compiled_path"));
        assert!(FINANCIAL_SCHEMA_USDA.contains("outputs:portfolio_wealth"));
        assert!(FINANCIAL_SCHEMA_USDA.contains("outputs:weights"));
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
