//! Bridge Plugin_Blueprints / Graphy graphs to MarketLab's execution engine.
//!
//! - [`FinanceNodeMetadataProvider`] — Graphy node palette + validation metadata
//! - [`FinanceGraphAdapter`] — `GraphDescription` → [`StageGraphSnapshot`] (engine input)

mod chart_model;
mod chart_raster;
mod cold_path_write;
mod asset_data;
mod bulk_assets;
mod viewport_timeline;
mod compile;
mod compile_profile;
mod metadata;
mod provider;
mod snapshot;
mod stage_tree;
mod layer_resolution;
mod stage_variants;
mod sweep;
mod telemetry;
mod taxonomy_index;
mod types;
mod blueprint;
mod node_series_cache;
mod performance_analytics;
mod performance_export;
mod performance_report;
mod portfolio_pins;
mod series_pins;
mod sparkline_bitmap;
mod usd_persistence;

pub use blueprint::{
    finance_category_icon, finance_data_types_compatible, finance_display_label,
    finance_is_analytics_node, finance_is_price_asset_node,
    finance_is_reporting_node,
    finance_node_header_rgba,
    finance_node_layout_extra_height, finance_node_graph_title, finance_primary_output_pin, finance_property_defaults,
    finance_property_fields, finance_property_is_numeric, finance_property_triggers_compile,
    finance_resolve_stream_pin,
    finance_remap_stream_input_pin,
    finance_remap_stream_output_pin,
    finance_is_stream_input_pin,
    finance_is_stream_output_pin,
    is_marketlab_finance_node, merge_finance_node_metadata, FinancePropertyField,
    FINANCE_SIGNAL_TYPE,
};

pub use metadata::finance_node_catalog;
pub use provider::FinanceNodeMetadataProvider;
pub use asset_data::{
    finance_asset_previews_for_snapshot, load_finance_asset_preview,
    load_finance_asset_preview_for_node, load_finance_return_asset_preview,
    normalize_finance_file_path, returns_to_ohlc_bars, FinanceAssetPreview, FinanceOhlcBar,
};
pub use bulk_assets::{
    collect_finance_bulk_drafts, finance_bulk_draft_from_csv_path,
    finance_bulk_draft_from_symbol, infer_csv_kind, list_bundled_finance_symbols,
    parse_symbol_tokens, FinanceBulkAssetDraft, FinanceCsvKind,
};
pub use viewport_timeline::{
    synthetic_bar_timestamps, ViewportAxisTick, ViewportTimelineBridge, ViewportYAxisMode,
};
pub use compile::{compile_finance_graph, FinanceCompileReport};
pub use compile_profile::{
    finance_compile_profile_to_sweep, FinanceCompileProfile,
};
pub use sweep::{
    run_finance_sweep, run_finance_sweep_with_profile, wealth_sparkline,
    FinancePortfolioSweepSummary, FinanceSweepResult,
};

pub use snapshot::{
    finance_node_prim_paths, graph_description_to_stage_snapshot, snapshot_for_engine_execution,
    FinanceGraphAdapter,
};
pub use stage_tree::{
    build_finance_stage_tree, filter_stage_tree_model, FinanceStageTreeModel, FinanceStageTreeRow,
};
pub use layer_resolution::{
    finance_property_layer_resolutions, finance_property_layer_resolutions_with_session,
    FinanceCompositionLayer, FinanceLayerContribution, FinancePropertyLayerResolution,
};
pub use stage_variants::{
    default_variant_token, finance_stage_variant_options, format_variant_label,
    resolve_variant_token, StageVariantOption,
};
pub use telemetry::{
    build_finance_workspace_telemetry, finance_fault_node_ids_from_warnings,
    finance_nodal_cache_health_pct, format_nodal_cache_gauge, FinanceDiagnosticState,
    FinanceWorkspaceTelemetry,
};
pub use types::{category, type_id, FinanceNodeKind, is_finance_price_asset_stage_type, PORTFOLIO_ALLOCATION_TOKENS};
pub use taxonomy_index::{
    finance_asset_properties_for_symbol, FinanceDatabaseIndex, TaxonomyIndex,
};
pub use cold_path_write::{
    prepare_finance_graph_for_cold_write, validate_finance_graph_for_cold_write,
    verify_cold_write_round_trip, FinanceColdWriteReport,
};
pub use usd_persistence::{
    export_document, import_document, stage_open_counter, FinanceLayerRef, FinanceSessionContext,
    FinanceWorkspaceDocument, UsdPersistenceError, UsdTransaction,
};
pub use node_series_cache::{
    build_finance_node_series_cache, bundle_scrub_readout, classify_series_kind,
    resolve_upstream_series, FinanceNodeSeriesBuildContext, FinanceNodeSeriesBundle,
    FinanceSeriesKind, FinanceSeriesResolveContext, NodeValueSummary, ResolvedFinanceSeries,
};
pub use performance_analytics::{
    compare_to_benchmark, compute_performance_bundle, cumulative_return_index,
    drawdown_series_pct, summarize_performance, FinanceBenchmarkComparison,
    FinancePerformanceSeriesBundle, FinancePerformanceSummary,
};
pub use performance_export::{performance_report_html, performance_summary_csv};
pub use performance_report::{
    build_finance_performance_reports, FinancePerformanceBuildContext, FinancePerformanceReport,
};
pub use portfolio_pins::{
    compact_portfolio_signal_target_pins, is_portfolio_signal_pin,
    portfolio_signal_pin_count, portfolio_signal_pin_id, portfolio_signal_pin_index,
};
pub use series_pins::{
    compact_performance_series_target_pins, is_performance_series_pin,
    performance_series_pin_count, performance_series_pin_id, performance_series_pin_index,
    PERFORMANCE_BENCHMARK_PIN,
};
pub use sparkline_bitmap::{
    rasterize_asset_preview_sparkline, rasterize_close_sparkline, rasterize_series_sparkline,
    FinanceSparklineBitmap, FINANCE_ASSET_SPARKLINE_BLOCK_HEIGHT, FINANCE_NODE_SPARKLINE_BLOCK_HEIGHT,
    FINANCE_SPARKLINE_HEIGHT, FINANCE_SPARKLINE_WIDTH,
};
pub use chart_model::{
    build_analytics_trading_chart, build_asset_ohlc_chart, build_isolated_series_chart,
    build_performance_chart, build_portfolio_wealth_chart, build_otl_analytics_sparkline_model,
    build_sparkline_model, ma_crossover_periods_from_script,
    build_wealth_trading_chart, ChartLayer, ChartPaneKind, ChartPaneSpec, FinanceChartModel,
    CHART_BACK_RGB, CHART_BULL_RGB, CHART_BEAR_RGB,
};
pub use chart_raster::{
    rasterize_finance_chart_pane, rasterize_finance_chart_thumbnail, CHART_RASTER_MAX_WIDTH,
    CHART_RASTER_MIN_WIDTH,
};
pub use pulsar_marketlab_core::{StageGraphSnapshot, StageSweepProfile};
