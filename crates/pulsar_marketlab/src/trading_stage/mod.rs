//! Layer 1 — OpenUSD-inspired in-memory market stage (Phase B Pillar 1).

mod market_stage;

pub use market_stage::{
    analytics_prim_path, asset_prim_path, stage_time_from_bar_date, MarketPrim,
    MarketStage, MarketStagePathError, TimeSampledAttribute,
};

pub(crate) use market_stage::validate_stage_path;
