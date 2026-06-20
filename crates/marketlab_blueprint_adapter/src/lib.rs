//! Bridge Plugin_Blueprints / Graphy graphs to MarketLab's execution engine.
//!
//! - [`FinanceNodeMetadataProvider`] — Graphy node palette + validation metadata
//! - [`FinanceGraphAdapter`] — `GraphDescription` → [`StageGraphSnapshot`] (engine input)

mod cold_path_write;
mod asset_data;
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
mod portfolio_pins;
mod sparkline_bitmap;
mod usd_persistence;

pub use blueprint::{
    finance_category_icon, finance_data_types_compatible, finance_display_label,
    finance_has_strategy_channels, finance_is_analytics_node, finance_node_header_rgba,
    finance_node_layout_extra_height, finance_node_graph_title, finance_primary_output_pin, finance_property_defaults,
    finance_property_fields, finance_property_is_numeric, finance_strategy_channel_fields,
    is_marketlab_finance_node, merge_finance_node_metadata, FinancePropertyField,
    FINANCE_STRATEGY_BLOCK_HEIGHT, FINANCE_STRATEGY_CHANNELS,
    FINANCE_SIGNAL_TYPE,
};

pub use metadata::finance_node_catalog;
pub use provider::FinanceNodeMetadataProvider;
pub use asset_data::{
    finance_asset_previews_for_snapshot, load_finance_asset_preview, FinanceAssetPreview,
    FinanceOhlcBar,
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
    finance_node_prim_paths, graph_description_to_stage_snapshot, FinanceGraphAdapter,
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
pub use types::{category, type_id, FinanceNodeKind, PORTFOLIO_ALLOCATION_TOKENS};
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
pub use portfolio_pins::{
    compact_portfolio_signal_target_pins, is_portfolio_signal_pin,
    portfolio_signal_pin_count, portfolio_signal_pin_id, portfolio_signal_pin_index,
};
pub use sparkline_bitmap::{
    rasterize_asset_preview_sparkline, rasterize_close_sparkline, FinanceSparklineBitmap,
    FINANCE_ASSET_SPARKLINE_BLOCK_HEIGHT, FINANCE_SPARKLINE_HEIGHT, FINANCE_SPARKLINE_WIDTH,
};
pub use pulsar_marketlab_core::{StageGraphSnapshot, StageSweepProfile};
