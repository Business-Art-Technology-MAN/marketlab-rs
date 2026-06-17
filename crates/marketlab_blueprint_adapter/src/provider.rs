//! Graphy [`NodeMetadataProvider`] for MarketLab finance nodes.

use std::collections::HashMap;

use graphy::{NodeMetadata, NodeMetadataProvider};

use crate::metadata::finance_node_catalog;
use crate::types::category;

/// Registry of finance node metadata for Plugin_Blueprints / Graphy compilers.
#[derive(Clone, Debug)]
pub struct FinanceNodeMetadataProvider {
    nodes: HashMap<String, NodeMetadata>,
}

impl Default for FinanceNodeMetadataProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl FinanceNodeMetadataProvider {
    pub fn new() -> Self {
        Self {
            nodes: finance_node_catalog(),
        }
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }
}

impl NodeMetadataProvider for FinanceNodeMetadataProvider {
    fn get_node_metadata(&self, node_type: &str) -> Option<&NodeMetadata> {
        self.nodes.get(node_type)
    }

    fn get_all_nodes(&self) -> Vec<&NodeMetadata> {
        self.nodes.values().collect()
    }

    fn get_nodes_by_category(&self, category: &str) -> Vec<&NodeMetadata> {
        self.nodes
            .values()
            .filter(|meta| meta.category == category)
            .collect()
    }
}

impl FinanceNodeMetadataProvider {
    pub fn universe_nodes(&self) -> Vec<&NodeMetadata> {
        self.get_nodes_by_category(category::UNIVERSE)
    }

    pub fn analytics_nodes(&self) -> Vec<&NodeMetadata> {
        self.get_nodes_by_category(category::ANALYTICS)
    }

    pub fn portfolio_nodes(&self) -> Vec<&NodeMetadata> {
        self.get_nodes_by_category(category::PORTFOLIOS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::type_id;

    #[test]
    fn provider_resolves_asset_metadata() {
        let provider = FinanceNodeMetadataProvider::new();
        let meta = provider
            .get_node_metadata(type_id::FINANCIAL_ASSET)
            .expect("asset metadata");
        assert_eq!(meta.category, category::UNIVERSE);
    }
}
