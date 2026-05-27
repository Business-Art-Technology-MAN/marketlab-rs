//! Production bridge from [`MarketStage`] to OTL [`MarketProviderServices`].

mod production_provider;
pub mod usd_spike;

pub use production_provider::ProductionStageProvider;
pub use usd_spike::{fixture_path, SharedOpenUsdStage, UsdStageBridge};

/// Parse `/prim/path/attribute` into `(prim_path, attribute)`.
pub fn parse_stage_attribute_path(full_path: &str) -> Option<(&str, &str)> {
    if !full_path.starts_with('/') || full_path.ends_with('/') {
        return None;
    }
    let slash = full_path.rfind('/')?;
    if slash == 0 {
        return None;
    }
    let attribute = &full_path[slash + 1..];
    if attribute.is_empty() {
        return None;
    }
    Some((&full_path[..slash], attribute))
}
