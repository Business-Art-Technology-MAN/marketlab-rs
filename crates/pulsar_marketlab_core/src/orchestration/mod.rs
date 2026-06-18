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
    compile_unified_script, normalize_for_series_eval, resolve_otl_script_src,
    wrap_series_script_as_signal_source, OtlScriptContext,
};
pub use engine::{
    ComposedAssetMeta, ComputedAttributeStream, ComputedTokenStream, EvaluationContext,
    ExecutionNode, GraphCompileSpec, GraphCompileWire, GraphEngineError, MarketLabGraphEngine,
    PathBindingIndex, SignalTransformFn, StageGraphPrim, StageGraphSnapshot, StageSweepProfile,
    TimelineExecutionResult,
};
pub use portfolio::{
    compute_allocation_weights, AssetQuote, ClosureLegKind,
    DirectionalDistribution, PortfolioIntegrationResult, PortfolioTrackingFrame, SymbolicOtlClosure,
};
