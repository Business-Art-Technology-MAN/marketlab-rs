//! Layer 1 — OpenUSD-inspired in-memory market stage (Phase B Pillar 1).

mod market_stage;
pub mod scene;

pub use market_stage::{
    analytics_prim_path, asset_prim_path, portfolio_prim_path, stage_time_from_bar_date,
    MarketPrim, MarketStage, MarketStagePathError, StageRelationship, TimeSampledAttribute,
};
pub use scene::{
    classify_type_name, is_legacy_bucket_path, is_operational_instance_path, is_schema_template_prim,
    marketlab_leaf_path, nested_prim_path, prim_is_class_spec, prim_type_name,
    should_show_prim_in_stage_tree, ExecutablePrimKind, MARKETLAB_DEFAULT_PRIM, MARKETLAB_ROOT,
};

pub(crate) use market_stage::validate_stage_path;
