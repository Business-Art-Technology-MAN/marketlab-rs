//! Dual-layer portfolio wealth chart: NAV line plus optional strategy overlays.

use std::collections::HashMap;

use gpui::*;
use gpui_component::chart::LineChart;
use pulsar_marketlab_core::{
    ComputedAttributeStream, PortfolioIntegrationResult, PortfolioTrackingFrame,
};

use crate::ohlc_chart_pane::{paint_playhead_line, CHART_PLOT_INSET};

const NAV_STROKE: u32 = 0x34d399;
const PEAK_STROKE: u32 = 0x64748b;
const NAV_STROKE_PX: f32 = 2.0;
const DRAWDOWN_FILL: u32 = 0xef4444;
const SIGNAL_BUY: u32 = 0x22c55e;
const SIGNAL_SELL: u32 = 0xef4444;
const REGIME_BAND: u32 = 0x6366f1;
const SIGNAL_WEIGHT_DELTA_THRESHOLD: f64 = 0.12;
const PRICE_BASE_SHIFT_RATIO: f64 = 0.025;

/// Interactive overlay toggles for the portfolio wealth chart.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PortfolioChartOverlayToggles {
    pub drawdown_shading: bool,
    pub signal_markers: bool,
    pub regime_scale_shifts: bool,
}

impl PortfolioChartOverlayToggles {
    pub fn with_defaults() -> Self {
        Self {
            drawdown_shading: true,
            signal_markers: true,
            regime_scale_shifts: false,
        }
    }
}

/// Absolute price-base regime shift detected on an underlying asset series.
#[derive(Clone, Debug, PartialEq)]
pub struct PriceBaseShift {
    pub bar_index: usize,
    pub asset_id: String,
    pub prior_price: f64,
    pub new_price: f64,
}

/// Pre-computed timeline bundle for inspector chart rendering (no runtime integration).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PortfolioWealthChartSeries {
    pub wealth: Vec<f64>,
    pub peak_equity: Vec<f64>,
    pub drawdown: Vec<f64>,
    pub signal_events: Vec<SignalEventMarker>,
    pub price_base_shifts: Vec<PriceBaseShift>,
    pub bar_labels: Vec<SharedString>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SignalEventMarker {
    pub bar_index: usize,
    pub weight_delta: f64,
    pub is_increase: bool,
}

#[derive(Clone)]
struct WealthPoint {
    label: SharedString,
    nav: f64,
}

/// Build chart series from graph-engine portfolio output (called once per compile sweep).
pub fn build_portfolio_wealth_chart_series(
    integration: &PortfolioIntegrationResult,
    bar_labels: Option<Vec<String>>,
) -> PortfolioWealthChartSeries {
    let wealth = integration.wealth_series.clone();
    let (peak_equity, drawdown) = compute_drawdown_series(&wealth);
    let signal_events = detect_signal_events(&integration.tracking_matrix);
    let price_base_shifts = detect_price_base_shifts(&integration.tracking_matrix);
    let bar_labels = bar_labels
        .unwrap_or_else(|| (0..wealth.len()).map(|i| i.to_string()).collect())
        .into_iter()
        .map(SharedString::from)
        .collect();

    PortfolioWealthChartSeries {
        wealth,
        peak_equity,
        drawdown,
        signal_events,
        price_base_shifts,
        bar_labels,
    }
}

/// Fallback builder when only attribute streams are available (legacy / partial cache).
pub fn build_portfolio_wealth_chart_from_streams(
    streams: &[ComputedAttributeStream],
    portfolio_prim_path: &str,
    bar_labels: Option<Vec<String>>,
) -> Option<PortfolioWealthChartSeries> {
    let wealth: Vec<f64> = streams
        .iter()
        .find(|stream| {
            stream.prim_path == portfolio_prim_path
                && stream.attribute == "outputs:portfolio_wealth"
        })
        .map(|stream| {
            let mut samples = stream.samples.clone();
            samples.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
            samples.into_iter().map(|(_, value)| value).collect()
        })?;

    if wealth.is_empty() {
        return None;
    }

    let (peak_equity, drawdown) = compute_drawdown_series(&wealth);
    let mut tracking_rows = Vec::new();
    for stream in streams {
        if stream.prim_path != portfolio_prim_path {
            continue;
        }
        if let Some(asset_id) = stream
            .attribute
            .strip_prefix("outputs:tracking:altered_weight:")
        {
            for (bar, weight) in &stream.samples {
                tracking_rows.push(PortfolioTrackingFrame {
                    timestamp: *bar as i64,
                    asset_id: asset_id.to_string(),
                    closure_raw_weight: *weight,
                    altered_portfolio_weight: *weight,
                    current_nominal_price: 0.0,
                    calculated_units: 0.0,
                    investment_return: 0.0,
                });
            }
        }
    }
    tracking_rows.sort_by_key(|row| row.timestamp);

    let signal_events = if tracking_rows.is_empty() {
        Vec::new()
    } else {
        detect_signal_events(&tracking_rows)
    };

    let bar_labels = bar_labels
        .unwrap_or_else(|| (0..wealth.len()).map(|i| i.to_string()).collect())
        .into_iter()
        .map(SharedString::from)
        .collect();

    Some(PortfolioWealthChartSeries {
        wealth,
        peak_equity,
        drawdown,
        signal_events,
        price_base_shifts: Vec::new(),
        bar_labels,
    })
}

fn compute_drawdown_series(wealth: &[f64]) -> (Vec<f64>, Vec<f64>) {
    let mut peak_equity = Vec::with_capacity(wealth.len());
    let mut drawdown = Vec::with_capacity(wealth.len());
    let mut running_peak = 0.0_f64;
    for sample in wealth {
        running_peak = running_peak.max(*sample);
        peak_equity.push(running_peak);
        let dd = if running_peak > f64::EPSILON {
            ((running_peak - sample) / running_peak).max(0.0)
        } else {
            0.0
        };
        drawdown.push(dd);
    }
    (peak_equity, drawdown)
}

fn detect_signal_events(tracking: &[PortfolioTrackingFrame]) -> Vec<SignalEventMarker> {
    let mut by_bar: HashMap<i64, f64> = HashMap::new();
    for row in tracking {
        *by_bar.entry(row.timestamp).or_insert(0.0) += row.closure_raw_weight.abs();
    }
    let mut bars: Vec<i64> = by_bar.keys().copied().collect();
    bars.sort_unstable();

    let mut events = Vec::new();
    let mut prior: Option<f64> = None;
    for bar in bars {
        let weight = by_bar[&bar];
        if let Some(previous) = prior {
            let delta = weight - previous;
            if delta.abs() >= SIGNAL_WEIGHT_DELTA_THRESHOLD {
                events.push(SignalEventMarker {
                    bar_index: bar as usize,
                    weight_delta: delta,
                    is_increase: delta > 0.0,
                });
            }
        }
        prior = Some(weight);
    }
    events
}

fn detect_price_base_shifts(tracking: &[PortfolioTrackingFrame]) -> Vec<PriceBaseShift> {
    let mut by_asset: HashMap<String, Vec<(usize, f64)>> = HashMap::new();
    for row in tracking {
        if row.current_nominal_price <= f64::EPSILON {
            continue;
        }
        by_asset
            .entry(row.asset_id.clone())
            .or_default()
            .push((row.timestamp as usize, row.current_nominal_price));
    }

    let mut shifts = Vec::new();
    for (asset_id, mut samples) in by_asset {
        samples.sort_by_key(|(bar, _)| *bar);
        for window in samples.windows(2) {
            let (_prior_bar, prior_price) = window[0];
            let (bar, new_price) = window[1];
            if prior_price <= f64::EPSILON {
                continue;
            }
            let ratio = (new_price - prior_price).abs() / prior_price;
            if ratio >= PRICE_BASE_SHIFT_RATIO {
                shifts.push(PriceBaseShift {
                    bar_index: bar,
                    asset_id: asset_id.clone(),
                    prior_price,
                    new_price,
                });
            }
        }
    }
    shifts.sort_by_key(|shift| shift.bar_index);
    shifts
}

pub fn render_portfolio_wealth_chart<H: PortfolioChartHost>(
    series: &PortfolioWealthChartSeries,
    toggles: PortfolioChartOverlayToggles,
    playhead_index: usize,
    view: Entity<H>,
) -> AnyElement {
    if series.wealth.len() < 2 {
        return div()
            .h(px(180.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded_md()
            .bg(rgb(0x111114))
            .border_1()
            .border_color(rgb(0x222227))
            .text_size(px(10.0))
            .font_family("monospace")
            .text_color(rgb(0x71717a))
            .child("Portfolio wealth series warming up — run a graph compile sweep.")
            .into_any_element();
    }

    let end = playhead_index.min(series.wealth.len().saturating_sub(1));
    let line_data: Vec<WealthPoint> = series
        .bar_labels
        .iter()
        .zip(series.wealth.iter())
        .map(|(label, nav)| WealthPoint {
            label: label.clone(),
            nav: *nav,
        })
        .collect();

    let tick_margin = (line_data.len() / 6).max(1);
    let overlay_series = series.clone();
    let overlay_toggles = toggles;
    let playhead = end;
    let total_bars = series.wealth.len();
    let (nav_min, nav_max) = wealth_y_domain(&series.wealth);

    div()
        .flex_col()
        .gap_2()
        .child(
            div()
                .text_size(px(9.0))
                .font_family("monospace")
                .text_color(rgb(0x71717a))
                .child(format!(
                    "NAV timeline · {} bars · playhead {}/{} · ${nav_min:.0}–${nav_max:.0}",
                    series.wealth.len(),
                    playhead_index + 1,
                    series.wealth.len()
                )),
        )
        .child(render_overlay_toggle_row(toggles, view.clone()))
        .child(
            div()
                .h(px(200.0))
                .min_h(px(160.0))
                .rounded_md()
                .bg(rgb(0x0f0f12))
                .border_1()
                .border_color(rgb(0x222227))
                .relative()
                .overflow_hidden()
                .child(
                    div()
                        .size_full()
                        .child(
                            LineChart::new(line_data)
                                .x(|point| point.label.clone())
                                .y(|point| point.nav)
                                .linear()
                                .stroke(hsla(0.55, 0.85, 0.62, 1.0))
                                .tick_margin(tick_margin),
                        ),
                )
                .child(
                    canvas(
                        |bounds, _window, _cx| bounds,
                        move |bounds, _state, window, _cx| {
                            paint_nav_timeline_line(bounds, &overlay_series.wealth, total_bars, window);
                            paint_portfolio_chart_overlays(
                                bounds,
                                &overlay_series,
                                overlay_toggles,
                                playhead,
                                total_bars,
                                window,
                            );
                            paint_playhead_line(bounds, playhead, total_bars, window);
                        },
                    )
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full(),
                ),
        )
        .into_any_element()
}

pub trait PortfolioChartHost: Sized + 'static {
    fn set_portfolio_chart_overlay(
        &mut self,
        overlay: PortfolioChartOverlayKey,
        enabled: bool,
        cx: &mut Context<Self>,
    );
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PortfolioChartOverlayKey {
    DrawdownShading,
    SignalMarkers,
    RegimeScaleShifts,
}

fn render_overlay_toggle_row<H: PortfolioChartHost>(
    toggles: PortfolioChartOverlayToggles,
    view: Entity<H>,
) -> impl IntoElement {
    use gpui_component::checkbox::Checkbox;

    let row = |label: &str, checked: bool, key: PortfolioChartOverlayKey, view: Entity<H>| {
        let overlay_index = match key {
            PortfolioChartOverlayKey::DrawdownShading => 0usize,
            PortfolioChartOverlayKey::SignalMarkers => 1usize,
            PortfolioChartOverlayKey::RegimeScaleShifts => 2usize,
        };
        div()
            .flex()
            .items_center()
            .gap_1()
            .child(
                Checkbox::new(("portfolio-overlay", overlay_index))
                    .checked(checked)
                    .on_click({
                        let view = view.clone();
                        move |enabled, window, cx| {
                            let enabled = *enabled;
                            view.update(cx, |host, cx| {
                                host.set_portfolio_chart_overlay(key, enabled, cx);
                            });
                            let _ = window;
                        }
                    }),
            )
            .child(
                div()
                    .text_size(px(9.0))
                    .font_family("monospace")
                    .text_color(rgb(0x94a3b8))
                    .child(label.to_string()),
            )
    };

    div()
        .flex()
        .flex_wrap()
        .gap_3()
        .child(row(
            "Drawdown shading",
            toggles.drawdown_shading,
            PortfolioChartOverlayKey::DrawdownShading,
            view.clone(),
        ))
        .child(row(
            "Signal markers",
            toggles.signal_markers,
            PortfolioChartOverlayKey::SignalMarkers,
            view.clone(),
        ))
        .child(row(
            "Regime / scale shifts",
            toggles.regime_scale_shifts,
            PortfolioChartOverlayKey::RegimeScaleShifts,
            view,
        ))
}

/// Paint the NAV curve on the overlay canvas (aligned with playhead + overlays).
fn paint_nav_timeline_line(
    bounds: Bounds<Pixels>,
    wealth: &[f64],
    visible_len: usize,
    window: &mut Window,
) {
    if visible_len < 2 {
        return;
    }
    let end = visible_len.min(wealth.len());
    let slice = &wealth[..end];
    if slice.iter().all(|value| !value.is_finite()) {
        return;
    }

    let (y_min, y_max) = wealth_y_domain(slice);
    let y_span = (y_max - y_min).max(f64::EPSILON);
    let bar_count = end.max(1);

    let origin_x: f32 = bounds.origin.x.into();
    let origin_y: f32 = bounds.origin.y.into();
    let width: f32 = bounds.size.width.into();
    let height: f32 = bounds.size.height.into();
    let inset = CHART_PLOT_INSET;
    let plot_width = (width - inset * 2.0).max(1.0);
    let plot_height = (height - inset * 2.0).max(1.0);

    let x_for_index = |index: usize| -> f32 {
        if bar_count <= 1 {
            origin_x + inset + plot_width * 0.5
        } else {
            origin_x + inset + plot_width * index as f32 / (bar_count - 1) as f32
        }
    };

    let y_for_value = |value: f64| -> f32 {
        let normalized = (value - y_min) / y_span;
        origin_y + inset + plot_height - normalized as f32 * plot_height
    };

    let stroke = rgb(NAV_STROKE);
    let mut path = PathBuilder::stroke(px(NAV_STROKE_PX));
    let mut started = false;
    for (index, value) in slice.iter().enumerate() {
        if !value.is_finite() {
            continue;
        }
        let point = point(px(x_for_index(index)), px(y_for_value(*value)));
        if started {
            path.line_to(point);
        } else {
            path.move_to(point);
            started = true;
        }
    }
    if let Ok(path) = path.build() {
        window.paint_path(path, stroke);
    }
}

fn paint_portfolio_chart_overlays(
    bounds: Bounds<Pixels>,
    series: &PortfolioWealthChartSeries,
    toggles: PortfolioChartOverlayToggles,
    _playhead_index: usize,
    visible_len: usize,
    window: &mut Window,
) {
    if visible_len < 2 {
        return;
    }

    let (y_min, y_max) = wealth_y_domain(&series.wealth[..visible_len]);
    let y_span = (y_max - y_min).max(f64::EPSILON);
    let bar_count = series.wealth.len().max(1);

    let origin_x: f32 = bounds.origin.x.into();
    let origin_y: f32 = bounds.origin.y.into();
    let width: f32 = bounds.size.width.into();
    let height: f32 = bounds.size.height.into();
    let inset = CHART_PLOT_INSET;
    let plot_width = (width - inset * 2.0).max(1.0);
    let plot_height = (height - inset * 2.0).max(1.0);

    let x_for_index = |index: usize| -> f32 {
        if bar_count <= 1 {
            origin_x + inset + plot_width * 0.5
        } else {
            origin_x + inset + plot_width * index as f32 / (bar_count - 1) as f32
        }
    };

    let y_for_value = |value: f64| -> f32 {
        let normalized = (value - y_min) / y_span;
        origin_y + inset + plot_height - normalized as f32 * plot_height
    };

    if toggles.regime_scale_shifts {
        for shift in &series.price_base_shifts {
            if shift.bar_index >= visible_len {
                continue;
            }
            let x = x_for_index(shift.bar_index);
            let band_half = 3.0;
            let top = origin_y + inset;
            let bottom = origin_y + inset + plot_height;
            let mut fill = PathBuilder::fill();
            fill.move_to(point(px(x - band_half), px(top)));
            fill.line_to(point(px(x + band_half), px(top)));
            fill.line_to(point(px(x + band_half), px(bottom)));
            fill.line_to(point(px(x - band_half), px(bottom)));
            fill.close();
            if let Ok(path) = fill.build() {
                window.paint_path(path, hsla(0.67, 0.55, 0.62, 0.12));
            }
        }
    }

    if toggles.drawdown_shading {
        for bar in 0..visible_len {
            if series.drawdown.get(bar).copied().unwrap_or(0.0) <= f64::EPSILON {
                continue;
            }
            let wealth = series.wealth[bar];
            let peak = series.peak_equity[bar];
            let x0 = if bar == 0 {
                x_for_index(0)
            } else {
                (x_for_index(bar - 1) + x_for_index(bar)) * 0.5
            };
            let x1 = if bar + 1 >= bar_count {
                x_for_index(bar)
            } else {
                (x_for_index(bar) + x_for_index(bar + 1)) * 0.5
            };
            let y_wealth = y_for_value(wealth);
            let y_peak = y_for_value(peak);
            let top = y_peak.min(y_wealth);
            let bottom = y_peak.max(y_wealth);
            let mut fill = PathBuilder::fill();
            fill.move_to(point(px(x0), px(top)));
            fill.line_to(point(px(x1), px(top)));
            fill.line_to(point(px(x1), px(bottom)));
            fill.line_to(point(px(x0), px(bottom)));
            fill.close();
            if let Ok(path) = fill.build() {
                window.paint_path(path, hsla(0.0, 0.75, 0.55, 0.22));
            }
        }

        let peak_stroke = rgb(PEAK_STROKE);
        let mut peak_path = PathBuilder::stroke(px(1.0));
        let mut started = false;
        for bar in 0..visible_len {
            let peak = series.peak_equity[bar];
            let point = point(px(x_for_index(bar)), px(y_for_value(peak)));
            if started {
                peak_path.line_to(point);
            } else {
                peak_path.move_to(point);
                started = true;
            }
        }
        if let Ok(path) = peak_path.build() {
            window.paint_path(path, peak_stroke);
        }
    }

    if toggles.signal_markers {
        for event in &series.signal_events {
            if event.bar_index >= visible_len {
                continue;
            }
            let x = x_for_index(event.bar_index);
            let y = y_for_value(series.wealth[event.bar_index]);
            let color = if event.is_increase {
                rgb(SIGNAL_BUY)
            } else {
                rgb(SIGNAL_SELL)
            };
            paint_signal_dot(window, x, y, color);
            let mut tick = PathBuilder::stroke(px(1.0));
            tick.move_to(point(px(x), px(origin_y + inset)));
            tick.line_to(point(px(x), px(origin_y + inset + plot_height)));
            if let Ok(path) = tick.build() {
                window.paint_path(path, hsla(if event.is_increase { 0.33 } else { 0.0 }, 0.7, 0.55, 0.35));
            }
        }
    }
}

fn wealth_y_domain(values: &[f64]) -> (f64, f64) {
    let mut min_value = f64::INFINITY;
    let mut max_value = f64::NEG_INFINITY;
    for value in values {
        if value.is_finite() {
            min_value = min_value.min(*value);
            max_value = max_value.max(*value);
        }
    }
    if !min_value.is_finite() || !max_value.is_finite() {
        return (0.0, 1.0);
    }
    let span = (max_value - min_value).max(max_value.abs() * 0.01).max(1.0);
    (min_value - span * 0.06, max_value + span * 0.06)
}

fn paint_signal_dot(window: &mut Window, x: f32, y: f32, color: Rgba) {
    let radius = 3.5;
    let mut builder = PathBuilder::fill();
    builder.move_to(point(px(x), px(y - radius)));
    builder.line_to(point(px(x + radius), px(y)));
    builder.line_to(point(px(x), px(y + radius)));
    builder.line_to(point(px(x - radius), px(y)));
    builder.close();
    if let Ok(path) = builder.build() {
        window.paint_path(path, color);
    }
}
