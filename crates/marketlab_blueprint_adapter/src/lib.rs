//! Bridge Plugin_Blueprints / Graphy graphs to MarketLab's execution engine.
//!
//! - [`FinanceNodeMetadataProvider`] — Graphy node palette + validation metadata
//! - [`FinanceGraphAdapter`] — `GraphDescription` → [`StageGraphSnapshot`] (engine input)

mod compile;
mod metadata;
mod provider;
mod snapshot;
mod sweep;
mod types;
mod blueprint;

pub use blueprint::{
    finance_category_icon, finance_data_types_compatible, finance_display_label,
    finance_primary_output_pin, finance_property_defaults, finance_property_fields,
    is_marketlab_finance_node, merge_finance_node_metadata, FinancePropertyField,
    FINANCE_SIGNAL_TYPE,
};

pub use metadata::finance_node_catalog;
pub use provider::FinanceNodeMetadataProvider;
pub use compile::{compile_finance_graph, FinanceCompileReport};
pub use sweep::{
    run_finance_sweep, wealth_sparkline, FinancePortfolioSweepSummary, FinanceSweepResult,
};

pub use snapshot::{
    finance_node_prim_paths, graph_description_to_stage_snapshot, FinanceGraphAdapter,
};
pub use types::{category, type_id, FinanceNodeKind, PORTFOLIO_ALLOCATION_TOKENS};

pub use graphy::{GraphDescription, NodeMetadata, NodeMetadataProvider};
pub use pulsar_marketlab_core::StageGraphSnapshot;
