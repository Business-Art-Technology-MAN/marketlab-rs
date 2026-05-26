use gpui::*;
use gpui::prelude::FluentBuilder;
use gpui_component::chart::{CandlestickChart, LineChart};
use pulsar_marketlab::technical_analysis::{
    build_ta_chart_layers, compute_ta_all_outputs_with_params, MarketSeriesWindow, TaChartLayer,
    TaVisualRole,
};

#[derive(Debug, Clone)]
pub struct OhlcBar {
    pub date: String,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
}

#[derive(Clone, Default)]
pub struct OhlcChartPaneConfig {
    pub asset_name: Option<String>,
    pub bars: Vec<OhlcBar>,
    pub ta_indicator_id: Option<String>,
    pub ta_indicator_label: Option<String>,
    pub ta_lookback_period: Option<u32>,
    /// Global synchronized playhead bar index (0-based).
    pub playhead_index: Option<usize>,
}

#[derive(Clone)]
struct CandlePoint {
    date: SharedString,
    open: f64,
    high: f64,
    low: f64,
    close: f64,
}

#[derive(Clone)]
struct OverlayPoint {
    date: SharedString,
    value: f64,
}

fn overlay_colors() -> [Hsla; 6] {
    [
        hsla(0.75, 0.7, 0.65, 1.0),
        hsla(0.55, 0.85, 0.6, 1.0),
        hsla(0.12, 0.85, 0.62, 1.0),
        hsla(0.95, 0.75, 0.62, 1.0),
        hsla(0.35, 0.7, 0.68, 1.0),
        hsla(0.08, 0.65, 0.58, 1.0),
    ]
}

const BUY_SIGNAL_COLOR: u32 = 0x22c55e;
const SELL_SIGNAL_COLOR: u32 = 0xef4444;
pub const PLAYHEAD_COLOR: u32 = 0xf59e0b;
pub const PLAYHEAD_STROKE_PX: f32 = 1.5;
pub const CHART_PLOT_INSET: f32 = 8.0;

pub fn playhead_x_for_index(index: usize, bounds: Bounds<Pixels>, total_bars: usize) -> f32 {
    let origin_x: f32 = bounds.origin.x.into();
    let width: f32 = bounds.size.width.into();
    let plot_width = (width - CHART_PLOT_INSET * 2.0).max(1.0);
    let bar_count = total_bars.max(1);
    if bar_count <= 1 {
        origin_x + CHART_PLOT_INSET + plot_width * 0.5
    } else {
        origin_x + CHART_PLOT_INSET + plot_width * index as f32 / (bar_count - 1) as f32
    }
}

pub fn playhead_index_for_mouse_x(
    mouse_x: f32,
    bounds: Bounds<Pixels>,
    total_bars: usize,
) -> usize {
    if total_bars <= 1 {
        return 0;
    }
    let origin_x: f32 = bounds.origin.x.into();
    let width: f32 = bounds.size.width.into();
    let plot_width = (width - CHART_PLOT_INSET * 2.0).max(1.0);
    let relative = ((mouse_x - origin_x - CHART_PLOT_INSET) / plot_width).clamp(0.0, 1.0);
    let index = (relative * (total_bars - 1) as f32).round() as usize;
    index.min(total_bars - 1)
}

pub fn paint_playhead_line(
    bounds: Bounds<Pixels>,
    playhead_index: usize,
    total_bars: usize,
    window: &mut Window,
) {
    if total_bars < 2 {
        return;
    }
    let x = playhead_x_for_index(playhead_index, bounds, total_bars);
    let origin_y: f32 = bounds.origin.y.into();
    let height: f32 = bounds.size.height.into();
    let stroke = rgb(PLAYHEAD_COLOR);
    let mut builder = PathBuilder::stroke(px(PLAYHEAD_STROKE_PX));
    builder.move_to(point(px(x), px(origin_y)));
    builder.line_to(point(px(x), px(origin_y + height)));
    if let Ok(path) = builder.build() {
        window.paint_path(path, stroke);
    }
}

pub fn render_ohlc_candlestick_pane(config: OhlcChartPaneConfig) -> AnyElement {
    let mut title = match (&config.asset_name, &config.ta_indicator_label) {
        (Some(asset), Some(ta)) => format!("OHLC // {asset} + {ta}"),
        (Some(asset), None) => format!("OHLC // {asset}"),
        (None, Some(ta)) => format!("OHLC // {ta}"),
        (None, None) => "OHLC Chart".to_string(),
    };
    if let Some(index) = config.playhead_index {
        if let Some(bar) = config.bars.get(index) {
            title = format!("{title} · bar {}/{} · {}", index + 1, config.bars.len(), bar.date);
        }
    }

    let header = chart_header(&title);

    if config.bars.len() < 2 {
        let hint = match (&config.asset_name, config.ta_indicator_id.is_some()) {
            (Some(_), true) | (None, true) => {
                "Wire this TA node to a CSV asset with OHLC data"
            }
            (Some(_), false) => "Selected asset has no OHLC columns in its CSV",
            (None, false) => "Select a CSV asset or Technical Analysis node",
        };
        empty_pane(header, hint).into_any_element()
    } else {
        render_ohlc_chart_body(config, header).into_any_element()
    }
}

fn render_ohlc_chart_body(
    config: OhlcChartPaneConfig,
    header: impl IntoElement,
) -> impl IntoElement {
    let bars = config.bars;
    let playhead_end = config
        .playhead_index
        .unwrap_or_else(|| bars.len().saturating_sub(1))
        .min(bars.len().saturating_sub(1));
    let bounded_bars = if bars.is_empty() {
        &bars[..]
    } else {
        &bars[..=playhead_end]
    };
    let ta_layers = config
        .ta_indicator_id
        .as_deref()
        .and_then(|indicator_id| {
            build_ta_layers(
                indicator_id,
                bounded_bars,
                config.ta_lookback_period.unwrap_or(14) as usize,
            )
        })
        .unwrap_or_default();
    let playhead_index = config.playhead_index;
    let total_bars = bars.len();
    let has_oscillator = ta_layers
        .iter()
        .any(|layer| layer.role == TaVisualRole::Oscillator);

    let candle_data: Vec<CandlePoint> = bars
        .iter()
        .map(|bar| CandlePoint {
            date: bar.date.clone().into(),
            open: bar.open,
            high: bar.high,
            low: bar.low,
            close: bar.close,
        })
        .collect();
    let tick_margin = (candle_data.len() / 8).max(1);

    let price_overlays: Vec<TaChartLayer> = ta_layers
        .iter()
        .filter(|layer| layer.role == TaVisualRole::PriceOverlay)
        .cloned()
        .collect();
    let signal_layers: Vec<TaChartLayer> = ta_layers
        .iter()
        .filter(|layer| matches!(layer.role, TaVisualRole::BuySignal | TaVisualRole::SellSignal))
        .cloned()
        .collect();
    let oscillator_layers: Vec<TaChartLayer> = ta_layers
        .iter()
        .filter(|layer| layer.role == TaVisualRole::Oscillator)
        .cloned()
        .collect();
    let overlay_bars = bars.clone();

    div()
        .flex_1()
        .flex()
        .flex_col()
        .min_h_0()
        .bg(rgb(0x0f0f12))
        .border_b_1()
        .border_color(rgb(0x2d2d34))
        .child(header)
        .child(
            div()
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .p_2()
                .gap_2()
                .child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .relative()
                        .min_h(px(160.0))
                        .child(
                            CandlestickChart::new(candle_data)
                                .x(|point| point.date.clone())
                                .open(|point| point.open)
                                .high(|point| point.high)
                                .low(|point| point.low)
                                .close(|point| point.close)
                                .tick_margin(tick_margin),
                        )
                        .child(
                            canvas(
                                |bounds, _window, _cx| bounds,
                                move |bounds, _state, window, _cx| {
                                    paint_ta_overlays(
                                        bounds,
                                        &overlay_bars,
                                        &price_overlays,
                                        &signal_layers,
                                        window,
                                    );
                                    if let Some(index) = playhead_index {
                                        paint_playhead_line(bounds, index, total_bars, window);
                                    }
                                },
                            )
                            .absolute()
                            .top_0()
                            .left_0()
                            .size_full(),
                        ),
                )
                .when(has_oscillator, |pane| {
                    pane.child(render_oscillator_panel(
                        bounded_bars,
                        &oscillator_layers,
                        tick_margin,
                    ))
                })
                .when(!ta_layers.is_empty(), |pane| pane.child(render_ta_legend(&ta_layers))),
        )
}

fn chart_header(title: &str) -> impl IntoElement {
    div()
        .px_3()
        .py_2()
        .bg(rgb(0x1c1c21))
        .border_b_1()
        .border_color(rgb(0x2d2d34))
        .text_xs()
        .font_weight(FontWeight::BOLD)
        .text_color(rgb(0xa1a1aa))
        .child(title.to_string())
}

fn empty_pane(header: impl IntoElement, hint: &str) -> impl IntoElement {
    div()
        .flex_1()
        .flex()
        .flex_col()
        .min_h_0()
        .bg(rgb(0x0f0f12))
        .border_b_1()
        .border_color(rgb(0x2d2d34))
        .child(header)
        .child(
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_color(rgb(0x52525b))
                .text_size(px(11.0))
                .child(hint.to_string()),
        )
}

fn build_ta_layers(
    indicator_id: &str,
    bars: &[OhlcBar],
    lookback: usize,
) -> Option<Vec<TaChartLayer>> {
    let mut window = MarketSeriesWindow::default();
    for bar in bars {
        window.push_bar(bar.open, bar.high, bar.low, bar.close, 0.0);
    }
    let outputs = compute_ta_all_outputs_with_params(indicator_id, &window, lookback)?;
    let price_range = bar_price_range(bars);
    Some(build_ta_chart_layers(
        indicator_id,
        &outputs,
        bars.len(),
        price_range,
    ))
}

fn bar_price_range(bars: &[OhlcBar]) -> Option<(f64, f64)> {
    let mut min_price = f64::INFINITY;
    let mut max_price = f64::NEG_INFINITY;
    for bar in bars {
        min_price = min_price.min(bar.low);
        max_price = max_price.max(bar.high);
    }
    if min_price.is_finite() && max_price.is_finite() {
        Some((min_price, max_price))
    } else {
        None
    }
}

fn paint_ta_overlays(
    bounds: Bounds<Pixels>,
    bars: &[OhlcBar],
    price_overlays: &[TaChartLayer],
    signal_layers: &[TaChartLayer],
    window: &mut Window,
) {
    if bars.is_empty() {
        return;
    }

    let (y_min, y_max) = overlay_y_domain(bars, price_overlays);
    let y_span = (y_max - y_min).max(f64::EPSILON);
    let bar_count = bars.len().max(1);

    let origin_x: f32 = bounds.origin.x.into();
    let origin_y: f32 = bounds.origin.y.into();
    let width: f32 = bounds.size.width.into();
    let height: f32 = bounds.size.height.into();
    let inset = 8.0;
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

    for layer in price_overlays {
        let colors = overlay_colors();
        let color = colors[layer.color_index % colors.len()];
        let stroke = color;
        let mut builder = PathBuilder::stroke(px(1.5));
        let mut started = false;
        for (index, value) in layer.values.iter().enumerate() {
            let Some(value) = value else {
                started = false;
                continue;
            };
            let point = point(px(x_for_index(index)), px(y_for_value(*value)));
            if started {
                builder.line_to(point);
            } else {
                builder.move_to(point);
                started = true;
            }
        }
        if let Ok(path) = builder.build() {
            window.paint_path(path, stroke);
        }
    }

    for layer in signal_layers {
        let color = match layer.role {
            TaVisualRole::SellSignal => rgb(SELL_SIGNAL_COLOR),
            _ => rgb(BUY_SIGNAL_COLOR),
        };
        for (index, value) in layer.values.iter().enumerate() {
            if !matches!(value, Some(v) if *v > 0.0) {
                continue;
            }
            let bar = &bars[index];
            let x = x_for_index(index);
            let y = match layer.role {
                TaVisualRole::SellSignal => y_for_value(bar.low),
                _ => y_for_value(bar.high),
            };
            paint_signal_marker(window, x, y, color, layer.role == TaVisualRole::BuySignal);
        }
    }
}

fn overlay_y_domain(bars: &[OhlcBar], overlays: &[TaChartLayer]) -> (f64, f64) {
    let mut min_value = bars
        .iter()
        .map(|bar| bar.low)
        .fold(f64::INFINITY, f64::min);
    let mut max_value = bars
        .iter()
        .map(|bar| bar.high)
        .fold(f64::NEG_INFINITY, f64::max);

    for layer in overlays {
        for value in layer.values.iter().flatten() {
            min_value = min_value.min(*value);
            max_value = max_value.max(*value);
        }
    }

    let span = (max_value - min_value).max(1.0);
    (min_value - span * 0.05, max_value + span * 0.05)
}

fn paint_signal_marker(window: &mut Window, x: f32, y: f32, color: Rgba, is_buy: bool) {
    let size = 4.0;
    let mut builder = PathBuilder::fill();
    if is_buy {
        builder.move_to(point(px(x), px(y - size)));
        builder.line_to(point(px(x - size), px(y + size)));
        builder.line_to(point(px(x + size), px(y + size)));
    } else {
        builder.move_to(point(px(x), px(y + size)));
        builder.line_to(point(px(x - size), px(y - size)));
        builder.line_to(point(px(x + size), px(y - size)));
    }
    builder.close();
    if let Ok(path) = builder.build() {
        window.paint_path(path, color);
    }
}

fn render_oscillator_panel(
    bars: &[OhlcBar],
    layers: &[TaChartLayer],
    tick_margin: usize,
) -> impl IntoElement {
    let primary = layers.first();
    let title = primary
        .map(|layer| format!("Oscillator // {}", layer.label))
        .unwrap_or_else(|| "Oscillator".to_string());

    let mut panel = div()
        .h(px(120.0))
        .flex_col()
        .gap_1()
        .child(
            div()
                .text_size(px(8.0))
                .font_family("monospace")
                .text_color(rgb(0x71717a))
                .child(title),
        );

    for layer in layers.iter().take(2) {
        let data: Vec<OverlayPoint> = bars
            .iter()
            .zip(layer.values.iter())
            .filter_map(|(bar, value)| {
                value.map(|value| OverlayPoint {
                    date: bar.date.clone().into(),
                    value,
                })
            })
            .collect();
        if data.len() < 2 {
            continue;
        }
        let colors = overlay_colors();
        let stroke = colors[layer.color_index % colors.len()];
        panel = panel.child(
            LineChart::new(data)
                .x(|point| point.date.clone())
                .y(|point| point.value)
                .linear()
                .stroke(stroke)
                .tick_margin(tick_margin.max(4)),
        );
    }

    panel
}

fn render_ta_legend(layers: &[TaChartLayer]) -> impl IntoElement {
    let mut legend = div().flex().flex_wrap().gap_2();
    let colors = overlay_colors();
    for layer in layers {
        let (swatch, role_label) = match layer.role {
            TaVisualRole::PriceOverlay => (
                colors[layer.color_index % colors.len()],
                "overlay",
            ),
            TaVisualRole::Oscillator => (
                colors[layer.color_index % colors.len()],
                "oscillator",
            ),
            TaVisualRole::BuySignal => (hsla(0.33, 0.7, 0.5, 1.0), "buy"),
            TaVisualRole::SellSignal => (hsla(0.0, 0.7, 0.55, 1.0), "sell"),
        };
        legend = legend.child(
            div()
                .flex()
                .items_center()
                .gap_1()
                .child(div().w_2().h_2().rounded_full().bg(swatch))
                .child(
                    div()
                        .text_size(px(8.0))
                        .font_family("monospace")
                        .text_color(rgb(0x71717a))
                        .child(format!("{} ({role_label})", layer.label)),
                ),
        );
    }
    legend
}
