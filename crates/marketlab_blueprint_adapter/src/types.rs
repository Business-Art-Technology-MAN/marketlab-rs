//! Finance node type identifiers and taxonomy aligned with MarketLab USD schema.

/// Palette / registry categories (matches stage composer scopes).
pub mod category {
    pub const UNIVERSE: &str = "Universe";
    pub const ANALYTICS: &str = "Analytics";
    pub const PORTFOLIOS: &str = "Portfolios";
    pub const REPORTING: &str = "Reporting";
}

/// Graphy `node_type` strings for finance nodes.
pub mod type_id {
    pub const FINANCIAL_ASSET: &str = "marketlab.universe.financial_asset";
    pub const FINANCIAL_RETURN_ASSET: &str = "marketlab.universe.financial_return_asset";
    pub const OTL_OPERATOR: &str = "marketlab.analytics.otl_operator";
    pub const TA_TREND: &str = "marketlab.analytics.ta_trend";
    pub const TA_VOLATILITY: &str = "marketlab.analytics.ta_volatility";
    pub const TA_OSCILLATOR: &str = "marketlab.analytics.ta_oscillator";
    pub const TA_CHANNEL: &str = "marketlab.analytics.ta_channel";
    pub const PORTFOLIO_INTEGRATOR: &str = "marketlab.portfolio.integrator";
    pub const PERFORMANCE_ANALYTICS: &str = "marketlab.analytics.performance_analytics";
}

/// Maps to [`pulsar_marketlab_core`] stage prim `type_name` values.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum FinanceNodeKind {
    FinancialAsset,
    FinancialReturnAsset,
    OtlOperator,
    OtlTaUberSignal,
    PortfolioIntegrator,
    PerformanceAnalytics,
}

impl FinanceNodeKind {
    pub fn stage_schema_type(self) -> &'static str {
        match self {
            Self::FinancialAsset => "FinancialAsset",
            Self::FinancialReturnAsset => "FinancialReturnAsset",
            Self::OtlOperator => "OtlOperator",
            Self::OtlTaUberSignal => "OtlTaUberSignal",
            Self::PortfolioIntegrator => "PortfolioIntegrator",
            Self::PerformanceAnalytics => "PerformanceAnalytics",
        }
    }

    pub fn graphy_type_id(self) -> &'static str {
        match self {
            Self::FinancialAsset => type_id::FINANCIAL_ASSET,
            Self::FinancialReturnAsset => type_id::FINANCIAL_RETURN_ASSET,
            Self::OtlOperator => type_id::OTL_OPERATOR,
            Self::OtlTaUberSignal => type_id::TA_TREND,
            Self::PortfolioIntegrator => type_id::PORTFOLIO_INTEGRATOR,
            Self::PerformanceAnalytics => type_id::PERFORMANCE_ANALYTICS,
        }
    }

    pub fn from_graphy_type_id(type_id: &str) -> Option<Self> {
        match type_id {
            type_id::FINANCIAL_ASSET => Some(Self::FinancialAsset),
            type_id::FINANCIAL_RETURN_ASSET => Some(Self::FinancialReturnAsset),
            type_id::OTL_OPERATOR => Some(Self::OtlOperator),
            type_id::TA_TREND
            | type_id::TA_VOLATILITY
            | type_id::TA_OSCILLATOR
            | type_id::TA_CHANNEL => Some(Self::OtlTaUberSignal),
            type_id::PORTFOLIO_INTEGRATOR => Some(Self::PortfolioIntegrator),
            type_id::PERFORMANCE_ANALYTICS => Some(Self::PerformanceAnalytics),
            _ => None,
        }
    }

    pub fn from_stage_type_name(type_name: &str) -> Option<Self> {
        match type_name {
            "FinancialAsset" => Some(Self::FinancialAsset),
            "FinancialReturnAsset" => Some(Self::FinancialReturnAsset),
            "OtlOperator" => Some(Self::OtlOperator),
            "OtlTaUberSignal" => Some(Self::OtlTaUberSignal),
            "PortfolioIntegrator" => Some(Self::PortfolioIntegrator),
            "PerformanceAnalytics" => Some(Self::PerformanceAnalytics),
            _ => None,
        }
    }

    pub fn is_price_source(self) -> bool {
        matches!(self, Self::FinancialAsset | Self::FinancialReturnAsset)
    }

    /// Nodes that emit a scalar series consumable by downstream OTL/TA (`close`, `result`, `wealth`).
    pub fn is_series_source(self) -> bool {
        self.is_price_source() || matches!(self, Self::PortfolioIntegrator | Self::OtlOperator | Self::OtlTaUberSignal)
    }

    pub fn ta_archetype_token(type_id: &str) -> Option<&'static str> {
        match type_id {
            type_id::TA_TREND => Some("trend"),
            type_id::TA_VOLATILITY => Some("volatility"),
            type_id::TA_OSCILLATOR => Some("oscillator"),
            type_id::TA_CHANNEL => Some("channel"),
            _ => None,
        }
    }
}

/// Stage prim types that contribute close-price vectors to the engine sweep.
pub fn is_finance_price_asset_stage_type(type_name: &str) -> bool {
    matches!(type_name, "FinancialAsset" | "FinancialReturnAsset")
}

/// Portfolio allocation methods exposed in schema.usda.
pub const PORTFOLIO_ALLOCATION_TOKENS: &[&str] = &[
    "Allocation::HierarchicalRiskParity",
    "Allocation::EqualWeight",
    "Allocation::MeanVariance",
];
