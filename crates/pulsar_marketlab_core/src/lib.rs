//! Core MarketLab resources: OpenUSD financial schema and shared constants.

/// Canonical financial schema layer (`FinancialAsset`, `OtlOperator`, `PortfolioIntegrator`).
pub const FINANCIAL_SCHEMA_USDA: &str = include_str!("../resources/usd/schema.usda");

#[cfg(test)]
mod tests {
    use super::FINANCIAL_SCHEMA_USDA;

    #[test]
    fn schema_defines_financial_typed_classes() {
        for class in ["FinancialAsset", "OtlOperator", "PortfolioIntegrator"] {
            assert!(
                FINANCIAL_SCHEMA_USDA.contains(&format!("class \"{class}\"")),
                "missing typed class {class}"
            );
        }
        assert!(FINANCIAL_SCHEMA_USDA.contains("inputs:symbol"));
        assert!(FINANCIAL_SCHEMA_USDA.contains("inputs:script_src"));
        assert!(FINANCIAL_SCHEMA_USDA.contains("outputs:portfolio_wealth"));
    }
}
