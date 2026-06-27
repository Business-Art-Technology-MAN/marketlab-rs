//! Viewport coordinate mapping between bar index, wall-clock labels, and chart pixels.
//!
//! Keeps timeline metadata on the warm UI path; the engine hot path stays index-only.

use std::ops::Range;

use crate::asset_data::FinanceAssetPreview;

/// How the Y-axis should label and scale plotted values.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ViewportYAxisMode {
    /// Raw price / USD from OHLC or wealth series.
    AbsolutePrice,
    /// Percent change from the first visible bar: `(P_t / P_start - 1) * 100`.
    CumulativeReturn,
    /// Normalized signal strength (e.g. OTL/TA overlays).
    SignalStrength,
}

/// X-axis label granularity chosen from the visible bar count.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum XAxisTimeFormat {
    Year,
    YearMonth,
    MonthDay,
    MonthDayTime,
}

/// One labeled tick on the X axis (bar index in full-series space).
#[derive(Clone, Debug, PartialEq)]
pub struct ViewportAxisTick {
    pub bar_index: usize,
    pub label: String,
}

/// Warm-path bridge: bar index ↔ timestamp string ↔ viewport pixel.
#[derive(Clone, Debug, PartialEq)]
pub struct ViewportTimelineBridge {
    /// Wall-clock (or synthetic) labels aligned 1:1 with the full bar series.
    pub string_timestamps: Vec<String>,
    /// Min/max Y values for the visible window after mode normalization.
    pub value_bounds: (f64, f64),
    /// Horizontal zoom window in bar/index space (end exclusive).
    pub visible_index_range: Range<usize>,
    pub y_axis_mode: ViewportYAxisMode,
}

impl ViewportTimelineBridge {
    pub fn for_asset_preview(preview: &FinanceAssetPreview, visible_range: Range<usize>) -> Self {
        let (min, max) = price_bounds_for_range(&preview.bars, visible_range.clone());
        Self {
            string_timestamps: preview.string_timestamps.clone(),
            value_bounds: (min, max),
            visible_index_range: visible_range,
            y_axis_mode: ViewportYAxisMode::AbsolutePrice,
        }
    }

    pub fn for_numeric_series(
        values: &[f64],
        timestamps: &[String],
        visible_range: Range<usize>,
        y_axis_mode: ViewportYAxisMode,
    ) -> Self {
        let bounds = value_bounds_for_range(values, visible_range.clone(), y_axis_mode);
        Self {
            string_timestamps: if timestamps.len() == values.len() {
                timestamps.to_vec()
            } else {
                synthetic_bar_timestamps(values.len())
            },
            value_bounds: bounds,
            visible_index_range: visible_range,
            y_axis_mode,
        }
    }

    pub fn with_y_axis_mode(mut self, mode: ViewportYAxisMode, values: &[f64]) -> Self {
        self.y_axis_mode = mode;
        self.value_bounds =
            value_bounds_for_range(values, self.visible_index_range.clone(), mode);
        self
    }

    pub fn visible_len(&self) -> usize {
        self.visible_index_range.end.saturating_sub(self.visible_index_range.start)
    }

    /// Maps a pixel X coordinate (relative to plot area origin) back to a bar index.
    pub fn pixel_to_index(&self, pixel_x: f32, plot_width: f32) -> usize {
        if plot_width <= f32::EPSILON {
            return self.visible_index_range.start;
        }
        let pct = (pixel_x / plot_width).clamp(0.0, 1.0) as f64;
        let start = self.visible_index_range.start as f64;
        let end = self.visible_index_range.end.saturating_sub(1) as f64;
        let index = start + pct * (end - start);
        index.round() as usize
    }

    /// Readable wall-clock (or synthetic) label for a pixel X in the plot area.
    pub fn pixel_to_timestamp(&self, pixel_x: f32, plot_width: f32) -> Option<&str> {
        let idx = self.pixel_to_index(pixel_x, plot_width);
        self.string_timestamps.get(idx).map(String::as_str)
    }

    /// Pixel X for a bar index within the plot area (0..plot_width).
    pub fn index_to_pixel(&self, bar_index: usize, plot_width: f32) -> f32 {
        let len = self.visible_len().max(1);
        let start = self.visible_index_range.start;
        let rel = bar_index.saturating_sub(start) as f32 / len as f32;
        rel * plot_width
    }

    pub fn x_axis_ticks(&self) -> Vec<ViewportAxisTick> {
        let visible_len = self.visible_len();
        if visible_len == 0 {
            return Vec::new();
        }
        let (stride, format) = x_stride_and_format(visible_len);
        let mut ticks = Vec::new();
        let start = self.visible_index_range.start;
        let end = self.visible_index_range.end;
        let mut index = start;
        loop {
            let label = self
                .string_timestamps
                .get(index)
                .map(|raw| format_timestamp_label(raw, format))
                .unwrap_or_else(|| index.to_string());
            ticks.push(ViewportAxisTick {
                bar_index: index,
                label,
            });
            let next = index.saturating_add(stride);
            if next >= end {
                break;
            }
            index = next;
        }
        ticks
    }

    pub fn y_axis_labels(&self, tick_count: usize) -> Vec<(f64, String)> {
        let count = tick_count.max(2);
        let (min, max) = self.value_bounds;
        if !min.is_finite() || !max.is_finite() {
            return Vec::new();
        }
        (0..count)
            .map(|step| {
                let t = step as f64 / (count - 1) as f64;
                let value = min + t * (max - min);
                (value, format_y_tick(value, self.y_axis_mode))
            })
            .collect()
    }
}

pub fn synthetic_bar_timestamps(len: usize) -> Vec<String> {
    (0..len).map(|index| format!("Bar {index}")).collect()
}

fn price_bounds_for_range(
    bars: &[crate::asset_data::FinanceOhlcBar],
    range: Range<usize>,
) -> (f64, f64) {
    let mut min = f64::INFINITY;
    let mut max = f64::NEG_INFINITY;
    for bar in bars.get(range.clone()).unwrap_or(&[]) {
        min = min.min(bar.low);
        max = max.max(bar.high);
    }
    if min.is_finite() && max.is_finite() {
        (min, max)
    } else {
        (0.0, 1.0)
    }
}

fn value_bounds_for_range(
    values: &[f64],
    range: Range<usize>,
    mode: ViewportYAxisMode,
) -> (f64, f64) {
    let slice: Vec<f64> = values
        .get(range.clone())
        .unwrap_or(&[])
        .iter()
        .copied()
        .collect();
    if slice.is_empty() {
        return (0.0, 1.0);
    }
    match mode {
        ViewportYAxisMode::AbsolutePrice | ViewportYAxisMode::SignalStrength => {
            let mut min = f64::INFINITY;
            let mut max = f64::NEG_INFINITY;
            for value in &slice {
                min = min.min(*value);
                max = max.max(*value);
            }
            if min.is_finite() && max.is_finite() {
                (min, max)
            } else {
                (0.0, 1.0)
            }
        }
        ViewportYAxisMode::CumulativeReturn => {
            let baseline = slice[0].abs().max(f64::EPSILON);
            let mut min = f64::INFINITY;
            let mut max = f64::NEG_INFINITY;
            for value in &slice {
                let pct = (*value / baseline - 1.0) * 100.0;
                min = min.min(pct);
                max = max.max(pct);
            }
            if min.is_finite() && max.is_finite() {
                (min, max)
            } else {
                (-1.0, 1.0)
            }
        }
    }
}

fn x_stride_and_format(visible_len: usize) -> (usize, XAxisTimeFormat) {
    if visible_len > 500 {
        let (stride, format) = if visible_len > 1000 {
            (126, XAxisTimeFormat::Year)
        } else {
            (63, XAxisTimeFormat::YearMonth)
        };
        (stride, format)
    } else if visible_len >= 100 {
        (21, XAxisTimeFormat::MonthDay)
    } else {
        (5, XAxisTimeFormat::MonthDayTime)
    }
}

fn format_timestamp_label(raw: &str, format: XAxisTimeFormat) -> String {
    let trimmed = raw.trim();
    if trimmed.starts_with("Bar ") {
        return trimmed.to_string();
    }

    let (date_part, time_part) = split_date_time(trimmed);
    match format {
        XAxisTimeFormat::Year => year_from_date(date_part),
        XAxisTimeFormat::YearMonth => {
            if date_part.len() >= 7 {
                date_part[..7].to_string()
            } else {
                year_from_date(date_part)
            }
        }
        XAxisTimeFormat::MonthDay => {
            if date_part.len() >= 10 {
                date_part[5..10].to_string()
            } else {
                date_part.to_string()
            }
        }
        XAxisTimeFormat::MonthDayTime => {
            let day = if date_part.len() >= 10 {
                &date_part[5..10]
            } else {
                date_part
            };
            if let Some(time) = time_part {
                format!("{day} {time}")
            } else {
                day.to_string()
            }
        }
    }
}

fn split_date_time(raw: &str) -> (&str, Option<&str>) {
    if let Some((date, time)) = raw.split_once('T') {
        return (date, Some(time.trim_end_matches('Z')));
    }
    if let Some((date, time)) = raw.split_once(' ') {
        if time.contains(':') {
            return (date, Some(time));
        }
    }
    (raw, None)
}

fn year_from_date(date_part: &str) -> String {
    if date_part.len() >= 4 {
        date_part[..4].to_string()
    } else {
        date_part.to_string()
    }
}

fn format_y_tick(value: f64, mode: ViewportYAxisMode) -> String {
    match mode {
        ViewportYAxisMode::AbsolutePrice => format!("${value:.2}"),
        ViewportYAxisMode::CumulativeReturn => format!("{value:+.1}%"),
        ViewportYAxisMode::SignalStrength => format!("{value:.2}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asset_data::{FinanceAssetPreview, FinanceOhlcBar};

    fn sample_preview() -> FinanceAssetPreview {
        FinanceAssetPreview {
            symbol: "SPY".to_string(),
            source_path: None,
            loaded_from_csv: true,
            synthetic: false,
            warnings: Vec::new(),
            bars: vec![
                FinanceOhlcBar {
                    open: 100.0,
                    high: 101.0,
                    low: 99.0,
                    close: 100.5,
                },
                FinanceOhlcBar {
                    open: 100.5,
                    high: 102.0,
                    low: 100.0,
                    close: 101.0,
                },
            ],
            string_timestamps: vec!["2024-01-02".to_string(), "2024-01-03".to_string()],
        }
    }

    #[test]
    fn pixel_to_index_interpolates_visible_range() {
        let bridge = ViewportTimelineBridge::for_asset_preview(&sample_preview(), 0..2);
        assert_eq!(bridge.pixel_to_index(0.0, 100.0), 0);
        assert_eq!(bridge.pixel_to_index(100.0, 100.0), 1);
    }

    #[test]
    fn x_axis_stride_macro_view_uses_year_month() {
        let timestamps: Vec<String> = (0..600)
            .map(|index| format!("2020-{:02}-{:02}", (index % 12) + 1, (index % 28) + 1))
            .collect();
        let bridge = ViewportTimelineBridge::for_numeric_series(
            &vec![1.0; 600],
            &timestamps,
            0..600,
            ViewportYAxisMode::AbsolutePrice,
        );
        let ticks = bridge.x_axis_ticks();
        assert!(ticks.len() >= 2);
        assert!(ticks[1].bar_index - ticks[0].bar_index >= 63);
        assert!(ticks[0].label.contains('-'));
    }

    #[test]
    fn cumulative_return_bounds_are_percentage() {
        let values = vec![100.0, 110.0, 105.0];
        let bridge = ViewportTimelineBridge::for_numeric_series(
            &values,
            &synthetic_bar_timestamps(3),
            0..3,
            ViewportYAxisMode::CumulativeReturn,
        );
        assert!((bridge.value_bounds.0 - 0.0).abs() < 1e-9);
        assert!((bridge.value_bounds.1 - 10.0).abs() < 1e-9);
    }
}
