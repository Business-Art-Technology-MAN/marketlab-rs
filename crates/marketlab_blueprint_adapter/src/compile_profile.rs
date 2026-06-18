//! Stage-tree compile profile: canvas node ids → engine [`StageSweepProfile`].

use std::collections::{HashMap, HashSet};

use pulsar_marketlab_core::StageSweepProfile;

/// Mute / solo / variant state from the Hydra stage tree (graph node ids).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FinanceCompileProfile {
    pub muted_node_ids: HashSet<String>,
    pub solo_node_id: Option<String>,
    /// Runtime variant overrides keyed by canvas node id (e.g. allocation method token).
    pub node_variant_overrides: HashMap<String, String>,
}

impl FinanceCompileProfile {
    pub fn is_active(&self) -> bool {
        !self.muted_node_ids.is_empty()
            || self.solo_node_id.is_some()
            || !self.node_variant_overrides.is_empty()
    }

    pub fn summary_lines(&self, node_prim_paths: &HashMap<String, String>) -> Vec<String> {
        if !self.is_active() {
            return Vec::new();
        }
        let mut lines = Vec::new();
        if !self.muted_node_ids.is_empty() {
            let labels: Vec<String> = self
                .muted_node_ids
                .iter()
                .filter_map(|id| node_prim_paths.get(id).cloned())
                .collect();
            lines.push(format!("Muted prims: {}", labels.join(", ")));
        }
        if let Some(solo_id) = &self.solo_node_id {
            if let Some(path) = node_prim_paths.get(solo_id) {
                lines.push(format!("Solo prim: {path}"));
            }
        }
        lines
    }
}

/// Map UI node ids to absolute prim paths for the engine sweep profile.
pub fn finance_compile_profile_to_sweep(
    profile: &FinanceCompileProfile,
    node_prim_paths: &HashMap<String, String>,
) -> StageSweepProfile {
    let muted_prim_paths = profile
        .muted_node_ids
        .iter()
        .filter_map(|id| node_prim_paths.get(id).cloned())
        .collect();
    let solo_prim_path = profile
        .solo_node_id
        .as_ref()
        .and_then(|id| node_prim_paths.get(id).cloned());
    let mut allocation_overrides = HashMap::new();
    for (node_id, token) in &profile.node_variant_overrides {
        if !token.starts_with("Allocation::") {
            continue;
        }
        let Some(path) = node_prim_paths.get(node_id) else {
            continue;
        };
        allocation_overrides.insert(path.clone(), token.clone());
    }
    StageSweepProfile {
        muted_prim_paths,
        solo_prim_path,
        allocation_overrides,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_node_ids_to_prim_paths() {
        let profile = FinanceCompileProfile {
            muted_node_ids: HashSet::from(["ta1".to_string()]),
            solo_node_id: Some("fund".to_string()),
            node_variant_overrides: HashMap::new(),
        };
        let paths = HashMap::from([
            ("ta1".to_string(), "/MarketLab/Analytics/ta1".to_string()),
            ("fund".to_string(), "/MarketLab/Portfolios/fund".to_string()),
        ]);
        let sweep = finance_compile_profile_to_sweep(&profile, &paths);
        assert!(sweep
            .muted_prim_paths
            .contains("/MarketLab/Analytics/ta1"));
        assert_eq!(
            sweep.solo_prim_path.as_deref(),
            Some("/MarketLab/Portfolios/fund")
        );
    }

    #[test]
    fn maps_variant_overrides_to_allocation_prim_paths() {
        let profile = FinanceCompileProfile {
            muted_node_ids: HashSet::new(),
            solo_node_id: None,
            node_variant_overrides: HashMap::from([(
                "fund".to_string(),
                "Allocation::MeanVariance".to_string(),
            )]),
        };
        let paths = HashMap::from([(
            "fund".to_string(),
            "/MarketLab/Portfolios/fund".to_string(),
        )]);
        let sweep = finance_compile_profile_to_sweep(&profile, &paths);
        assert_eq!(
            sweep
                .allocation_overrides
                .get("/MarketLab/Portfolios/fund")
                .map(String::as_str),
            Some("Allocation::MeanVariance")
        );
    }
}
