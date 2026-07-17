//! Unified finance chart model for Hydra, Performance, and EventGraph thumbnails.

use std::ops::Range;

use crate::asset_data::{FinanceAssetPreview, FinanceOhlcBar};
use crate::node_series_cache::{FinanceNodeSeriesBundle, FinanceSeriesKind, NodeValueSummary};
use crate::performance_report::FinancePerformanceReport;
use crate::sweep::FinancePortfolioSweepSummary;

const DEFAULT_VISIBLE_BARS: usize = 96;

pub const CHART_BULL_RGB: u32 = 0x26a69a;
pub const CHART_BEAR_RGB: u32 = 0xef5350;
pub const CHART_WEALTH_RGB: u32 = 0xa855f7;
pub const CHART_INDICATOR_RGB: u32 = 0x4a9eff;
pub const CHART_GRID_RGB: u32 = 0x2a2e39;
pub const CHART_BACK_RGB: u32 = 0x131722;
pub const CHART_GATE_LONG_RGB: u32 = 0x26a69a;
pub const CHART_GATE_SHORT_RGB: u32 = 0xef5350;
pub const CHART_GATE_FLAT_RGB: u32 = 0x52525b;
pub const CHART_LONG_SHADE_RGB: u32 = 0x26a69a;
pub const CHART_MA_FAST_RGB: u32 = 0xf5c542;
pub const CHART_MA_SLOW_RGB: u32 = 0xc084fc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChartPaneKind {
    MainOhlc,
    MainLine,
    Volume,
    Indicator,
    Gate,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ChartLayer {
    pub id: String,
    pub label: String,
    pub kind: FinanceSeriesKind,
    pub values: Vec<f64>,
    pub color_rgb: u32,
    /// When true, draw on the main pane as an overlay line.
    pub overlay: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ChartPaneSpec {
    pub kind: ChartPaneKind,
    pub weight: f32,
    pub layer_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FinanceChartModel {
    pub title: String,
    pub subtitle: String,
    pub bars: Option<Vec<FinanceOhlcBar>>,
    pub timestamps: Vec<String>,
    pub layers: Vec<ChartLayer>,
    pub panes: Vec<ChartPaneSpec>,
    pub visible_range: Range<usize>,
    pub crosshair_bar: usize,
    pub gate_series: Option<Vec<f64>>,
    pub summary: NodeValueSummary,
}

impl FinanceChartModel {
    pub fn total_bars(&self) -> usize {
        self.timestamps
            .len()
            .max(self.bars.as_ref().map(|b| b.len()).unwrap_or(0))
            .max(
                self.layers
                    .iter()
                    .map(|layer| layer.values.len())
                    .max()
                    .unwrap_or(0),
            )
    }

    pub fn layer(&self, id: &str) -> Option<&ChartLayer> {
        self.layers.iter().find(|layer| layer.id == id)
    }

    pub fn tail_visible_range(total_bars: usize, max_bars: usize) -> Range<usize> {
        if total_bars <= max_bars {
            return 0..total_bars;
        }
        total_bars - max_bars..total_bars
    }
}

pub fn build_asset_ohlc_chart(
    preview: &FinanceAssetPreview,
    title: &str,
    visible_range: Range<usize>,
    crosshair_bar: usize,
    indicator: Option<(&str, &[f64])>,
    gate: Option<&[f64]>,
    summary: &NodeValueSummary,
) -> FinanceChartModel {
    let mut layers = Vec::new();
    if let Some((label, values)) = indicator {
        layers.push(ChartLayer {
            id: "indicator".to_string(),
            label: label.to_string(),
            kind: FinanceSeriesKind::Indicator,
            values: values.to_vec(),
            color_rgb: CHART_INDICATOR_RGB,
            overlay: true,
        });
    }
    let panes = asset_trading_panes(indicator.is_some(), gate.is_some());
    FinanceChartModel {
        title: title.to_string(),
        subtitle: format!(
            "{} · {} bars",
            preview.symbol,
            preview.bars.len()
        ),
        bars: Some(preview.bars.clone()),
        timestamps: preview.string_timestamps.clone(),
        layers,
        panes,
        visible_range,
        crosshair_bar,
        gate_series: gate.map(|values| values.to_vec()),
        summary: summary.clone(),
    }
}

pub fn build_wealth_trading_chart(
    title: &str,
    wealth: &[f64],
    timestamps: &[String],
    visible_range: Range<usize>,
    crosshair_bar: usize,
    indicator: Option<(&str, &[f64])>,
    gate: Option<&[f64]>,
    summary: &NodeValueSummary,
) -> FinanceChartModel {
    let mut layers = vec![ChartLayer {
        id: "wealth".to_string(),
        label: "NAV".to_string(),
        kind: FinanceSeriesKind::Wealth,
        values: wealth.to_vec(),
        color_rgb: CHART_WEALTH_RGB,
        overlay: false,
    }];
    if let Some((label, values)) = indicator {
        layers.push(ChartLayer {
            id: "indicator".to_string(),
            label: label.to_string(),
            kind: FinanceSeriesKind::Indicator,
            values: values.to_vec(),
            color_rgb: CHART_INDICATOR_RGB,
            overlay: true,
        });
    }
    let panes = wealth_trading_panes(indicator.is_some(), gate.is_some());
    FinanceChartModel {
        title: title.to_string(),
        subtitle: format!("{} bars · last ${:.2}", wealth.len(), summary.last),
        bars: None,
        timestamps: timestamps.to_vec(),
        layers,
        panes,
        visible_range,
        crosshair_bar,
        gate_series: gate.map(|values| values.to_vec()),
        summary: summary.clone(),
    }
}

pub fn build_analytics_trading_chart(
    bundle: &FinanceNodeSeriesBundle,
    node_label: &str,
    input_kind: FinanceSeriesKind,
    preview: Option<&FinanceAssetPreview>,
    input_wealth: Option<&[f64]>,
    input_timestamps: &[String],
    visible_range: Range<usize>,
    crosshair_bar: usize,
) -> FinanceChartModel {
    let indicator = (bundle.primary_kind == FinanceSeriesKind::Indicator)
        .then_some(bundle.primary_series.as_slice());
    let gate = (bundle.primary_kind == FinanceSeriesKind::Gate)
        .then_some(bundle.primary_series.as_slice());

    match input_kind {
        FinanceSeriesKind::Price => {
            let preview = preview.expect("price preview required");
            build_asset_ohlc_chart(
                preview,
                &format!("{node_label} · TA"),
                visible_range,
                crosshair_bar,
                indicator.map(|s| ("Signal", s)),
                gate,
                &bundle.summary,
            )
        }
        FinanceSeriesKind::Wealth => {
            let wealth = input_wealth.expect("wealth series required");
            build_wealth_trading_chart(
                &format!("{node_label} · TA"),
                wealth,
                input_timestamps,
                visible_range,
                crosshair_bar,
                indicator.map(|s| ("Signal", s)),
                gate,
                &bundle.summary,
            )
        }
        _ => build_isolated_series_chart(
            node_label,
            &bundle.primary_series,
            bundle.primary_kind,
            visible_range,
            crosshair_bar,
            &bundle.summary,
        ),
    }
}

pub fn build_portfolio_wealth_chart(
    portfolio: &FinancePortfolioSweepSummary,
    timestamps: &[String],
    visible_range: Range<usize>,
    crosshair_bar: usize,
) -> FinanceChartModel {
    let summary = NodeValueSummary {
        min: portfolio
            .wealth_series
            .iter()
            .copied()
            .fold(f64::INFINITY, f64::min),
        max: portfolio
            .wealth_series
            .iter()
            .copied()
            .fold(f64::NEG_INFINITY, f64::max),
        last: portfolio.final_wealth,
        long_pct: None,
        flat_pct: None,
        short_pct: None,
    };
    build_wealth_trading_chart(
        &portfolio.label,
        &portfolio.wealth_series,
        timestamps,
        visible_range,
        crosshair_bar,
        None,
        None,
        &summary,
    )
}

pub fn build_performance_chart(
    report: &FinancePerformanceReport,
    visible_range: Range<usize>,
    crosshair_bar: usize,
) -> FinanceChartModel {
    let summary = NodeValueSummary {
        min: report
            .bundle
            .cumulative_return_pct
            .iter()
            .copied()
            .fold(f64::INFINITY, f64::min),
        max: report
            .bundle
            .cumulative_return_pct
            .iter()
            .copied()
            .fold(f64::NEG_INFINITY, f64::max),
        last: report.bundle.summary.total_return_pct,
        long_pct: None,
        flat_pct: None,
        short_pct: None,
    };
    let timestamps: Vec<String> = (0..report.bundle.cumulative_return_pct.len())
        .map(|index| index.to_string())
        .collect();
    let mut model = build_isolated_series_chart(
        &report.label,
        &report.bundle.cumulative_return_pct,
        FinanceSeriesKind::Wealth,
        visible_range,
        crosshair_bar,
        &summary,
    );
    model.subtitle = format!(
        "{:+.2}% total · Sharpe {:.2}",
        report.bundle.summary.total_return_pct, report.bundle.summary.sharpe
    );
    model.timestamps = timestamps;
    if let Some(layer) = model.layers.iter_mut().find(|l| l.id == "series") {
        layer.label = "Return %".to_string();
        layer.color_rgb = if report.bundle.summary.total_return_pct >= 0.0 {
            CHART_BULL_RGB
        } else {
            CHART_BEAR_RGB
        };
    }
    model
}

pub fn build_isolated_series_chart(
    title: &str,
    values: &[f64],
    kind: FinanceSeriesKind,
    visible_range: Range<usize>,
    crosshair_bar: usize,
    summary: &NodeValueSummary,
) -> FinanceChartModel {
    let color = match kind {
        FinanceSeriesKind::Gate => CHART_GATE_LONG_RGB,
        FinanceSeriesKind::Wealth => CHART_WEALTH_RGB,
        FinanceSeriesKind::Indicator => CHART_INDICATOR_RGB,
        FinanceSeriesKind::Price => CHART_BULL_RGB,
    };
    let panes = vec![ChartPaneSpec {
        kind: if kind == FinanceSeriesKind::Gate {
            ChartPaneKind::Gate
        } else {
            ChartPaneKind::MainLine
        },
        weight: 1.0,
        layer_ids: vec!["series".to_string()],
    }];
    FinanceChartModel {
        title: title.to_string(),
        subtitle: format!("last {:.4}", summary.last),
        bars: None,
        timestamps: (0..values.len()).map(|i| i.to_string()).collect(),
        layers: vec![ChartLayer {
            id: "series".to_string(),
            label: title.to_string(),
            kind,
            values: values.to_vec(),
            color_rgb: color,
            overlay: false,
        }],
        panes,
        visible_range,
        crosshair_bar,
        gate_series: if kind == FinanceSeriesKind::Gate {
            Some(values.to_vec())
        } else {
            None
        },
        summary: summary.clone(),
    }
}

pub fn build_sparkline_model(values: &[f64], kind: FinanceSeriesKind) -> FinanceChartModel {
    let len = values.len();
    let range = 0..len;
    let summary = NodeValueSummary {
        min: values.iter().copied().fold(f64::INFINITY, f64::min),
        max: values.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        last: *values.last().unwrap_or(&0.0),
        long_pct: None,
        flat_pct: None,
        short_pct: None,
    };
    build_isolated_series_chart("sparkline", values, kind, range, len.saturating_sub(1), &summary)
}

/// EventGraph OTL thumbnail: OHLC base, gate/signal overlay, and optional MA lines.
pub fn build_otl_analytics_sparkline_model(
    preview: &FinanceAssetPreview,
    bundle: &FinanceNodeSeriesBundle,
    label: &str,
    ma_fast: Option<usize>,
    ma_slow: Option<usize>,
) -> FinanceChartModel {
    use pulsar_marketlab_core::sma;

    let len = preview.bars.len();
    let range = FinanceChartModel::tail_visible_range(len, len);
    let crosshair = len.saturating_sub(1);
    let summary = NodeValueSummary {
        min: preview.bars.iter().map(|bar| bar.low).fold(f64::INFINITY, f64::min),
        max: preview.bars.iter().map(|bar| bar.high).fold(f64::NEG_INFINITY, f64::max),
        last: preview.bars.last().map(|bar| bar.close).unwrap_or(0.0),
        long_pct: bundle.summary.long_pct,
        flat_pct: bundle.summary.flat_pct,
        short_pct: bundle.summary.short_pct,
    };
    let indicator = (bundle.primary_kind == FinanceSeriesKind::Indicator)
        .then_some(bundle.primary_series.as_slice());
    let gate = (bundle.primary_kind == FinanceSeriesKind::Gate)
        .then_some(bundle.primary_series.as_slice());
    let mut model = build_asset_ohlc_chart(
        preview,
        label,
        range,
        crosshair,
        indicator.map(|series| ("Signal", series)),
        gate,
        &summary,
    );
    if let (Some(fast), Some(slow)) = (ma_fast, ma_slow) {
        if fast > 0 && slow > 0 {
            let closes = preview.close_series();
            model.layers.push(ChartLayer {
                id: "ma_fast".to_string(),
                label: format!("MA {fast}"),
                kind: FinanceSeriesKind::Indicator,
                values: sma(&closes, fast),
                color_rgb: CHART_MA_FAST_RGB,
                overlay: true,
            });
            model.layers.push(ChartLayer {
                id: "ma_slow".to_string(),
                label: format!("MA {slow}"),
                kind: FinanceSeriesKind::Indicator,
                values: sma(&closes, slow),
                color_rgb: CHART_MA_SLOW_RGB,
                overlay: true,
            });
        }
    }
    model
}

/// Read `fast` / `slow` defaults from an OSL `shader ma_crossover(...)` script header.
pub fn ma_crossover_periods_from_script(script: &str) -> Option<(usize, usize)> {
    let signature = pulsar_marketlab_core::parse_script_signature(script);
    let fast = signature
        .parameters
        .iter()
        .find(|param| param.name == "fast")
        .and_then(|param| param.default_value.map(|value| value.max(1.0) as usize))?;
    let slow = signature
        .parameters
        .iter()
        .find(|param| param.name == "slow")
        .and_then(|param| param.default_value.map(|value| value.max(1.0) as usize))?;
    Some((fast, slow))
}

fn asset_trading_panes(has_indicator: bool, has_gate: bool) -> Vec<ChartPaneSpec> {
    let mut panes = vec![
        ChartPaneSpec {
            kind: ChartPaneKind::MainOhlc,
            weight: if has_indicator || has_gate { 0.55 } else { 0.78 },
            layer_ids: vec!["indicator".to_string()],
        },
        ChartPaneSpec {
            kind: ChartPaneKind::Volume,
            weight: 0.12,
            layer_ids: vec![],
        },
    ];
    if has_indicator {
        panes.push(ChartPaneSpec {
            kind: ChartPaneKind::Indicator,
            weight: 0.18,
            layer_ids: vec!["indicator".to_string()],
        });
    }
    if has_gate {
        panes.push(ChartPaneSpec {
            kind: ChartPaneKind::Gate,
            weight: 0.15,
            layer_ids: vec![],
        });
    }
    normalize_pane_weights(panes)
}

fn wealth_trading_panes(has_indicator: bool, has_gate: bool) -> Vec<ChartPaneSpec> {
    let mut panes = vec![ChartPaneSpec {
        kind: ChartPaneKind::MainLine,
        weight: if has_indicator || has_gate { 0.58 } else { 0.82 },
        layer_ids: vec!["wealth".to_string(), "indicator".to_string()],
    }];
    if has_indicator {
        panes.push(ChartPaneSpec {
            kind: ChartPaneKind::Indicator,
            weight: 0.22,
            layer_ids: vec!["indicator".to_string()],
        });
    }
    if has_gate {
        panes.push(ChartPaneSpec {
            kind: ChartPaneKind::Gate,
            weight: 0.20,
            layer_ids: vec![],
        });
    }
    normalize_pane_weights(panes)
}

fn normalize_pane_weights(mut panes: Vec<ChartPaneSpec>) -> Vec<ChartPaneSpec> {
    let total: f32 = panes.iter().map(|pane| pane.weight).sum();
    if total > f32::EPSILON {
        for pane in &mut panes {
            pane.weight /= total;
        }
    }
    panes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asset_panes_include_volume_and_gate() {
        let panes = asset_trading_panes(true, true);
        assert_eq!(panes.len(), 4);
        assert!(panes.iter().any(|p| p.kind == ChartPaneKind::Volume));
        assert!(panes.iter().any(|p| p.kind == ChartPaneKind::Gate));
    }

    #[test]
    fn sparkline_model_uses_full_timeline() {
        let values: Vec<f64> = (0..200).map(|index| index as f64).collect();
        let model = build_sparkline_model(&values, FinanceSeriesKind::Gate);
        assert_eq!(model.visible_range, 0..200);
    }
}
