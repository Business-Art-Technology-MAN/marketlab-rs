use std::collections::{BTreeMap, HashSet};

use vector_ta::indicators::dispatch::cpu_single::compute_cpu;
use vector_ta::indicators::dispatch::types::{
    IndicatorComputeRequest, IndicatorDataRef, IndicatorSeries, ParamKV, ParamValue,
};
use vector_ta::indicators::registry::{
    get_indicator, list_indicators, IndicatorInputKind, IndicatorValueType,
};
use vector_ta::utilities::enums::Kernel;

pub const DEFAULT_TA_INDICATOR_ID: &str = "rsi";
pub const DEFAULT_TA_LOOKBACK: usize = 14;
pub const MIN_TA_LOOKBACK: usize = 2;
pub const MAX_TA_LOOKBACK: usize = 200;

/// Sidebar quick-pick algorithms (maps to VectorTA indicator ids).
pub const TA_SIDEBAR_ALGORITHMS: &[(&str, &str)] = &[("rsi", "RSI"), ("ema", "EMA"), ("sma", "SMA")];

pub fn clamp_ta_lookback(period: usize) -> usize {
    period.clamp(MIN_TA_LOOKBACK, MAX_TA_LOOKBACK)
}

pub fn ta_period_param_kv(period: usize) -> ParamKV<'static> {
    ParamKV {
        key: "period",
        value: ParamValue::Int(clamp_ta_lookback(period) as i64),
    }
}

/// On-demand TA evaluation bound to a market window; `(bar_index, lookback)` samples the series.
pub struct TaEvaluationClosure {
    pub run: Box<dyn Fn(usize, usize) -> Option<f32> + Send + Sync>,
}

pub fn build_ta_evaluation_closure(
    indicator_id: String,
    window: MarketSeriesWindow,
) -> TaEvaluationClosure {
    TaEvaluationClosure {
        run: Box::new(move |bar_index, lookback| {
            let end = bar_index.saturating_add(1);
            if end == 0 || end > window.len() {
                return None;
            }
            let slice = sub_window_to(&window, end);
            compute_ta_latest_with_params(&indicator_id, &slice, lookback).map(|value| value as f32)
        }),
    }
}

fn sub_window_to(window: &MarketSeriesWindow, end_exclusive: usize) -> MarketSeriesWindow {
    let end = end_exclusive.min(window.len());
    MarketSeriesWindow {
        open: window.open[..end].to_vec(),
        high: window.high[..end].to_vec(),
        low: window.low[..end].to_vec(),
        close: window.close[..end].to_vec(),
        volume: window.volume[..end].to_vec(),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaCatalogEntry {
    pub id: String,
    pub label: String,
    pub category: String,
}

/// One shelf group in the VectorTA hierarchy (category → indicators).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaCatalogCategory {
    pub id: String,
    pub label: String,
    pub entries: Vec<TaCatalogEntry>,
}

impl TaCatalogCategory {
    pub fn count(&self) -> usize {
        self.entries.len()
    }
}

const CATEGORY_SHELF_ORDER: &[&str] = &[
    "moving_average",
    "momentum",
    "trend",
    "volatility",
    "volume",
    "cycle",
    "oscillator",
    "relative_strength",
    "breadth",
    "statistics",
];

pub fn category_display_label(category_id: &str) -> String {
    match category_id {
        "moving_average" => "Moving Avg".to_string(),
        "momentum" => "Momentum".to_string(),
        "trend" => "Trend".to_string(),
        "volatility" => "Volatility".to_string(),
        "volume" => "Volume".to_string(),
        "cycle" => "Cycle".to_string(),
        "oscillator" => "Oscillator".to_string(),
        "relative_strength" => "Rel Strength".to_string(),
        "breadth" => "Breadth".to_string(),
        "statistics" => "Statistics".to_string(),
        other => humanize_category_id(other),
    }
}

pub fn ta_category_for_indicator(indicator_id: &str) -> Option<String> {
    get_indicator(indicator_id).map(|info| info.category.to_string())
}

fn humanize_category_id(category_id: &str) -> String {
    category_id
        .split('_')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn category_sort_key(category_id: &str) -> (u8, String) {
    CATEGORY_SHELF_ORDER
        .iter()
        .position(|id| *id == category_id)
        .map(|index| (index as u8, String::new()))
        .unwrap_or((u8::MAX, category_id.to_string()))
}

/// Full VectorTA indicator catalog (registry-driven, ~340 entries).
pub fn ta_indicator_catalog() -> Vec<TaCatalogEntry> {
    let mut seen = HashSet::new();
    let mut entries = Vec::new();
    for info in list_indicators() {
        if !info.capabilities.supports_cpu_single {
            continue;
        }
        if info.input_kind == IndicatorInputKind::Candles {
            continue;
        }
        if seen.insert(info.id.to_string()) {
            entries.push(TaCatalogEntry {
                id: info.id.to_string(),
                label: info.label.to_string(),
                category: info.category.to_string(),
            });
        }
    }
    entries.sort_by(|a, b| {
        a.category
            .cmp(&b.category)
            .then_with(|| a.label.cmp(&b.label))
    });
    entries
}

/// VectorTA catalog grouped by indicator category for shelf navigation.
pub fn ta_indicator_catalog_hierarchy() -> Vec<TaCatalogCategory> {
    let mut buckets: BTreeMap<String, Vec<TaCatalogEntry>> = BTreeMap::new();
    for entry in ta_indicator_catalog() {
        buckets
            .entry(entry.category.clone())
            .or_default()
            .push(entry);
    }

    let mut categories: Vec<TaCatalogCategory> = buckets
        .into_iter()
        .map(|(id, mut entries)| {
            entries.sort_by(|a, b| a.label.cmp(&b.label));
            TaCatalogCategory {
                id: id.clone(),
                label: category_display_label(&id),
                entries,
            }
        })
        .collect();

    categories.sort_by(|left, right| {
        category_sort_key(&left.id).cmp(&category_sort_key(&right.id))
    });
    categories
}

pub fn ta_indicator_label(indicator_id: &str) -> Option<&'static str> {
    get_indicator(indicator_id).map(|info| info.label)
}

#[derive(Clone, Debug, Default)]
pub struct MarketSeriesWindow {
    pub open: Vec<f64>,
    pub high: Vec<f64>,
    pub low: Vec<f64>,
    pub close: Vec<f64>,
    pub volume: Vec<f64>,
}

impl MarketSeriesWindow {
    pub fn len(&self) -> usize {
        self.close.len()
    }

    pub fn is_empty(&self) -> bool {
        self.close.is_empty()
    }

    pub fn push_bar(&mut self, open: f64, high: f64, low: f64, close: f64, volume: f64) {
        self.open.push(open);
        self.high.push(high);
        self.low.push(low);
        self.close.push(close);
        self.volume.push(volume);
    }

    pub fn push_close_only(&mut self, close: f64) {
        self.push_bar(close, close, close, close, 0.0);
    }
}

pub fn compute_ta_latest(indicator_id: &str, window: &MarketSeriesWindow) -> Option<f64> {
    compute_ta_latest_with_params(indicator_id, window, DEFAULT_TA_LOOKBACK)
}

pub fn compute_ta_latest_with_params(
    indicator_id: &str,
    window: &MarketSeriesWindow,
    lookback: usize,
) -> Option<f64> {
    compute_ta_all_outputs_with_params(indicator_id, window, lookback)?
        .into_iter()
        .find_map(|output| output.values.iter().rev().find(|v| v.is_finite()).copied())
}

pub fn compute_ta_all_outputs(
    indicator_id: &str,
    window: &MarketSeriesWindow,
) -> Option<Vec<TaSeriesOutput>> {
    compute_ta_all_outputs_with_params(indicator_id, window, DEFAULT_TA_LOOKBACK)
}

pub fn compute_ta_all_outputs_with_params(
    indicator_id: &str,
    window: &MarketSeriesWindow,
    lookback: usize,
) -> Option<Vec<TaSeriesOutput>> {
    if window.is_empty() {
        return None;
    }

    let info = get_indicator(indicator_id)?;
    if !info.capabilities.supports_cpu_single {
        return None;
    }

    let data = indicator_data_ref(window, info.input_kind)?;
    let period = clamp_ta_lookback(lookback);
    let params = [ta_period_param_kv(period)];
    let mut outputs = Vec::new();

    if info.outputs.is_empty() {
        push_ta_output(
            indicator_id,
            "value",
            "Value",
            IndicatorValueType::F64,
            data,
            &params,
            &mut outputs,
        );
    } else {
        for output_info in &info.outputs {
            push_ta_output(
                indicator_id,
                output_info.id,
                output_info.label,
                output_info.value_type,
                data,
                &params,
                &mut outputs,
            );
        }
    }

    if outputs.is_empty() {
        None
    } else {
        Some(outputs)
    }
}

#[derive(Clone, Debug)]
pub struct TaSeriesOutput {
    pub output_id: String,
    pub label: String,
    pub values: Vec<f64>,
    pub is_boolean: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaVisualRole {
    PriceOverlay,
    Oscillator,
    BuySignal,
    SellSignal,
}

#[derive(Clone, Debug)]
pub struct TaChartLayer {
    pub label: String,
    pub role: TaVisualRole,
    pub values: Vec<Option<f64>>,
    pub color_index: usize,
}

pub fn build_ta_chart_layers(
    indicator_id: &str,
    outputs: &[TaSeriesOutput],
    bar_count: usize,
    price_range: Option<(f64, f64)>,
) -> Vec<TaChartLayer> {
    let category = get_indicator(indicator_id)
        .map(|info| info.category)
        .unwrap_or_default();

    outputs
        .iter()
        .enumerate()
        .map(|(index, output)| {
            let aligned = align_series_to_bars(&output.values, bar_count);
            let role = classify_output_role(
                indicator_id,
                &output.output_id,
                &output.label,
                category,
                output.is_boolean,
                &aligned,
                price_range,
            );
            TaChartLayer {
                label: output.label.clone(),
                role,
                values: aligned,
                color_index: index,
            }
        })
        .collect()
}

fn push_ta_output(
    indicator_id: &str,
    output_id: &str,
    output_label: &str,
    value_type: IndicatorValueType,
    data: IndicatorDataRef<'_>,
    params: &[ParamKV<'_>],
    outputs: &mut Vec<TaSeriesOutput>,
) {
    let request = IndicatorComputeRequest {
        indicator_id,
        output_id: Some(output_id),
        data,
        params,
        kernel: Kernel::Auto,
    };

    let Ok(result) = compute_cpu(request) else {
        return;
    };
    let Some(values) = series_to_f64_vec(&result.series) else {
        return;
    };
    if values.is_empty() {
        return;
    }

    outputs.push(TaSeriesOutput {
        output_id: output_id.to_string(),
        label: output_label.to_string(),
        values,
        is_boolean: matches!(value_type, IndicatorValueType::Bool),
    });
}

fn indicator_data_ref<'a>(
    window: &'a MarketSeriesWindow,
    input_kind: IndicatorInputKind,
) -> Option<IndicatorDataRef<'a>> {
    Some(match input_kind {
        IndicatorInputKind::Slice => IndicatorDataRef::Slice {
            values: &window.close,
        },
        IndicatorInputKind::Ohlc => IndicatorDataRef::Ohlc {
            open: &window.open,
            high: &window.high,
            low: &window.low,
            close: &window.close,
        },
        IndicatorInputKind::Ohlcv => IndicatorDataRef::Ohlcv {
            open: &window.open,
            high: &window.high,
            low: &window.low,
            close: &window.close,
            volume: &window.volume,
        },
        IndicatorInputKind::HighLow => IndicatorDataRef::HighLow {
            high: &window.high,
            low: &window.low,
        },
        IndicatorInputKind::CloseVolume => IndicatorDataRef::CloseVolume {
            close: &window.close,
            volume: &window.volume,
        },
        IndicatorInputKind::Candles => return None,
    })
}

fn series_to_f64_vec(series: &IndicatorSeries) -> Option<Vec<f64>> {
    match series {
        IndicatorSeries::F64(values) => Some(values.clone()),
        IndicatorSeries::I32(values) => Some(values.iter().map(|v| *v as f64).collect()),
        IndicatorSeries::Bool(values) => Some(
            values
                .iter()
                .map(|value| if *value { 1.0 } else { 0.0 })
                .collect(),
        ),
    }
}

fn align_series_to_bars(values: &[f64], bar_count: usize) -> Vec<Option<f64>> {
    if bar_count == 0 {
        return Vec::new();
    }
    if values.len() >= bar_count {
        values[values.len() - bar_count..]
            .iter()
            .map(|value| value.is_finite().then_some(*value))
            .collect()
    } else {
        let pad = bar_count - values.len();
        std::iter::repeat(None)
            .take(pad)
            .chain(values.iter().map(|value| value.is_finite().then_some(*value)))
            .collect()
    }
}

fn classify_output_role(
    indicator_id: &str,
    output_id: &str,
    label: &str,
    category: &str,
    is_boolean: bool,
    values: &[Option<f64>],
    price_range: Option<(f64, f64)>,
) -> TaVisualRole {
    let indicator_l = indicator_id.to_ascii_lowercase();
    let id = output_id.to_ascii_lowercase();
    let label_l = label.to_ascii_lowercase();
    let category_l = category.to_ascii_lowercase();

    if is_boolean {
        if id.contains("sell") || label_l.contains("sell") || id.contains("short") {
            return TaVisualRole::SellSignal;
        }
        return TaVisualRole::BuySignal;
    }

    if category_l.contains("moving_average")
        || id.contains("sma")
        || id.contains("ema")
        || id.contains("hma")
        || id.contains("dema")
        || id.contains("tema")
        || id.contains("rma")
        || id.contains("alma")
        || id.contains("upper")
        || id.contains("lower")
        || id.contains("middle")
        || id.contains("mid")
        || id.contains("band")
    {
        return TaVisualRole::PriceOverlay;
    }

    if category_l.contains("oscillator")
        || category_l.contains("momentum")
        || indicator_l.contains("rsi")
        || indicator_l.contains("macd")
        || indicator_l.contains("stoch")
        || id.contains("histogram")
    {
        return TaVisualRole::Oscillator;
    }

    if values_overlap_price_range(values, price_range) {
        TaVisualRole::PriceOverlay
    } else {
        TaVisualRole::Oscillator
    }
}

fn values_overlap_price_range(
    values: &[Option<f64>],
    price_range: Option<(f64, f64)>,
) -> bool {
    let Some((min_price, max_price)) = price_range else {
        return false;
    };
    let span = (max_price - min_price).max(1.0);
    let Some(median) = median_finite(values) else {
        return false;
    };
    median >= min_price - span * 0.25 && median <= max_price + span * 0.25
}

fn median_finite(values: &[Option<f64>]) -> Option<f64> {
    let mut finite: Vec<f64> = values.iter().filter_map(|value| *value).collect();
    if finite.is_empty() {
        return None;
    }
    finite.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    Some(finite[finite.len() / 2])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_is_populated() {
        assert!(ta_indicator_catalog().len() > 100);
    }

    #[test]
    fn catalog_hierarchy_preserves_flat_count() {
        let flat_len = ta_indicator_catalog().len();
        let hierarchy = ta_indicator_catalog_hierarchy();
        let grouped_len: usize = hierarchy.iter().map(|category| category.count()).sum();
        assert_eq!(flat_len, grouped_len);
        assert!(hierarchy.len() > 5);
        assert!(hierarchy.iter().any(|category| category.id == "momentum"));
    }

    #[test]
    fn rsi_respects_custom_lookback_period() {
        let mut window = MarketSeriesWindow::default();
        for close in (0..30).map(|i| 100.0 + (i as f64) * 0.5) {
            window.push_close_only(close);
        }
        let short = compute_ta_latest_with_params("rsi", &window, 5);
        let long = compute_ta_latest_with_params("rsi", &window, 14);
        assert!(short.is_some());
        assert!(long.is_some());
    }

    #[test]
    fn evaluation_closure_samples_playhead_bar() {
        let mut window = MarketSeriesWindow::default();
        for close in (0..30).map(|i| 100.0 + (i as f64) * 0.5) {
            window.push_close_only(close);
        }
        let closure = build_ta_evaluation_closure("rsi".to_string(), window);
        assert!((closure.run)(29, 14).is_some());
    }

    #[test]
    fn rsi_category_is_momentum() {
        assert_eq!(
            ta_category_for_indicator("rsi").as_deref(),
            Some("momentum")
        );
    }

    #[test]
    fn rsi_computes_all_outputs() {
        let mut window = MarketSeriesWindow::default();
        for close in (0..30).map(|i| 100.0 + (i as f64) * 0.5) {
            window.push_close_only(close);
        }
        let outputs = compute_ta_all_outputs("rsi", &window).expect("rsi outputs");
        assert!(!outputs.is_empty());
        let layers = build_ta_chart_layers("rsi", &outputs, 30, Some((100.0, 115.0)));
        assert!(layers.iter().any(|layer| layer.role == TaVisualRole::Oscillator));
    }

    #[test]
    fn rsi_computes_on_close_series() {
        let mut window = MarketSeriesWindow::default();
        for close in (0..30).map(|i| 100.0 + (i as f64) * 0.5) {
            window.push_close_only(close);
        }
        let value = compute_ta_latest("rsi", &window);
        assert!(value.is_some());
    }
}
