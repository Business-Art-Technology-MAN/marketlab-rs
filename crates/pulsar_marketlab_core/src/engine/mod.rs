//! OTL Phase 2 vectorized timeline sweep.

mod market_timeline_window;
mod matrix_provider;
mod parallel_sweep;
mod snapshot;
mod vector_sweep;

pub use parallel_sweep::{
    compute_execution_levels, is_parallel_tier_signal, merge_parallel_signal_outcomes,
    run_parallel_signal_batch, ParallelSignalJob, ParallelSignalOutcome, ParallelSweepContext,
};
pub use market_timeline_window::{
    shared_columns_from_vec, MarketTimelineWindow, SharedPriceColumn, WeightTrackCallback,
};
pub use matrix_provider::{
    allocation_weights_from_covariance, fill_subcovariance_block, uses_covariance_optimizer,
    PrecomputedMatrixCache, RollingMatrixWindow, DEFAULT_COVARIANCE_LOOKBACK,
};
pub use snapshot::{
    chronological_stride, format_timeline_tick, HistoricalTimelineMap,
};
pub use vector_sweep::{evaluate_compiled_tier, evaluate_node_vector_series};