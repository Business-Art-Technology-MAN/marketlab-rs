//! USD stage graph compilation and timeline execution.

mod binary;
mod compiler;
mod engine;
mod portfolio;
mod script_resolve;

pub use binary::{
    deserialize_from_bytes, load_compiled_asset_from_path, manifest_json_from_signature,
    serialize_to_bytes, NodeManifest, OtcBinaryDecoder,
    OtcBinaryEncoder, OtcBinaryHeader, OtcCompiledAsset, OtcError, CURRENT_ENGINE_GENERATION,
};
pub use compiler::{
    compile, compile_script, compile_script_multi, compile_script_multi_with_context, cross,
    display_name_for_script, ema, macd, normalize_script_for_compile, parse,
    parse_script_entry_point_name, parse_script_scalar_uniforms, parse_script_signature,
    parse_with_context, rsi, set_script_uniform_default, sma, tokenize, BinOp, CompileError,
    CompiledSeries, Expr, MultiSeriesClosure, OslParamType, OslParameter, ScriptCompileContext,
    ScriptSignature, SeriesClosure, Token,
};
pub use script_resolve::{
    compile_unified_script, normalize_for_series_eval, resolve_otl_script_src, OtlScriptContext,
};
pub use engine::{
    ComputedAttributeStream, EvaluationContext, ExecutionNode, GraphCompileSpec, GraphCompileWire,
    GraphEngineError, MarketLabGraphEngine, SignalTransformFn, StageGraphPrim, StageGraphSnapshot,
    TimelineExecutionResult,
};
pub use portfolio::{
    integrate_portfolio, AssetQuote, BasePosition, ClosureLegKind, DirectionalDistribution,
    PortfolioIntegrationResult, PortfolioIntegratorConfig, PortfolioOtlState,
    PortfolioOtlTransformFn, PortfolioTrackingFrame, SymbolicOtlClosure,
};
