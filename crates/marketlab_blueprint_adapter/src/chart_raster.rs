//! CPU rasterization for finance chart panes (Hydra hero charts + thumbnails).

use std::ops::Range;

use crate::chart_model::{
    ChartPaneKind, FinanceChartModel, CHART_BACK_RGB, CHART_BEAR_RGB, CHART_BULL_RGB,
    CHART_GATE_FLAT_RGB, CHART_GATE_LONG_RGB, CHART_GATE_SHORT_RGB, CHART_GRID_RGB,
    CHART_LONG_SHADE_RGB,
};
use crate::sparkline_bitmap::FinanceSparklineBitmap;

pub const CHART_RASTER_MAX_WIDTH: u32 = 1280;
pub const CHART_RASTER_MIN_WIDTH: u32 = 320;

pub fn rasterize_finance_chart_pane(
    model: &FinanceChartModel,
    pane_index: usize,
    width: u32,
    height: u32,
) -> Option<FinanceSparklineBitmap> {
    if width == 0 || height == 0 {
        return None;
    }
    let pane = model.panes.get(pane_index)?;
    let mut rgba = vec![0u8; (width * height * 4) as usize];
    fill_rgb(&mut rgba, width, height, CHART_BACK_RGB);

    let plot_left = 4u32;
    let plot_top = 4u32;
    let plot_right = width.saturating_sub(4);
    let plot_bottom = height.saturating_sub(4);
    let plot_w = plot_right.saturating_sub(plot_left).max(1);
    let plot_h = plot_bottom.saturating_sub(plot_top).max(1);

    draw_grid(&mut rgba, width, plot_left, plot_top, plot_w, plot_h);

    let range = model.visible_range.clone();

    match pane.kind {
        ChartPaneKind::MainOhlc => {
            if let Some(bars) = &model.bars {
                let visible: Vec<_> = bars.get(range.clone()).unwrap_or(&[]).to_vec();
                draw_long_shading(
                    &mut rgba,
                    width,
                    model.gate_series.as_deref(),
                    &range,
                    plot_left,
                    plot_top,
                    plot_w,
                    plot_h,
                );
                draw_candles(
                    &mut rgba,
                    width,
                    &visible,
                    plot_left,
                    plot_top,
                    plot_w,
                    plot_h,
                );
                for layer_id in &pane.layer_ids {
                    if let Some(layer) = model.layer(layer_id) {
                        if layer.overlay {
                            draw_series_line(
                                &mut rgba,
                                width,
                                &layer.values,
                                &range,
                                plot_left,
                                plot_top,
                                plot_w,
                                plot_h,
                                layer.color_rgb,
                                price_bounds_for_bars(&visible),
                            );
                        }
                    }
                }
            }
        }
        ChartPaneKind::MainLine => {
            for layer_id in &pane.layer_ids {
                if let Some(layer) = model.layer(layer_id) {
                    if !layer.overlay {
                        draw_series_line(
                            &mut rgba,
                            width,
                            &layer.values,
                            &range,
                            plot_left,
                            plot_top,
                            plot_w,
                            plot_h,
                            layer.color_rgb,
                            value_bounds(&layer.values, &range),
                        );
                    } else {
                        draw_series_line(
                            &mut rgba,
                            width,
                            &layer.values,
                            &range,
                            plot_left,
                            plot_top,
                            plot_w,
                            plot_h,
                            layer.color_rgb,
                            main_line_bounds(model, &range),
                        );
                    }
                }
            }
        }
        ChartPaneKind::Volume => {
            if let Some(bars) = &model.bars {
                let visible: Vec<_> = bars.get(range.clone()).unwrap_or(&[]).to_vec();
                draw_volume_bars(
                    &mut rgba,
                    width,
                    &visible,
                    plot_left,
                    plot_top,
                    plot_w,
                    plot_h,
                );
            } else if let Some(layer) = model.layers.first() {
                draw_volume_from_series(
                    &mut rgba,
                    width,
                    &layer.values,
                    &range,
                    plot_left,
                    plot_top,
                    plot_w,
                    plot_h,
                );
            }
        }
        ChartPaneKind::Indicator => {
            for layer_id in &pane.layer_ids {
                if let Some(layer) = model.layer(layer_id) {
                    draw_series_line(
                        &mut rgba,
                        width,
                        &layer.values,
                        &range,
                        plot_left,
                        plot_top,
                        plot_w,
                        plot_h,
                        layer.color_rgb,
                        value_bounds(&layer.values, &range),
                    );
                }
            }
        }
        ChartPaneKind::Gate => {
            if let Some(gate) = model.gate_series.as_deref() {
                draw_gate_pane(
                    &mut rgba,
                    width,
                    gate,
                    &range,
                    plot_left,
                    plot_top,
                    plot_w,
                    plot_h,
                );
            }
        }
    }

    draw_crosshair(
        &mut rgba,
        width,
        model.crosshair_bar,
        &range,
        plot_left,
        plot_top,
        plot_w,
        plot_h,
    );

    Some(FinanceSparklineBitmap {
        width,
        height,
        rgba,
    })
}

pub fn rasterize_finance_chart_thumbnail(
    model: &FinanceChartModel,
    width: u32,
    height: u32,
) -> Option<FinanceSparklineBitmap> {
    let main_index = model
        .panes
        .iter()
        .position(|pane| {
            matches!(
                pane.kind,
                ChartPaneKind::MainOhlc | ChartPaneKind::MainLine | ChartPaneKind::Gate
            )
        })
        .unwrap_or(0);
    rasterize_finance_chart_pane(model, main_index, width, height)
}

fn main_line_bounds(model: &FinanceChartModel, range: &Range<usize>) -> (f64, f64) {
    let mut min = f64::INFINITY;
    let mut max = f64::NEG_INFINITY;
    for layer in &model.layers {
        if let Some((lmin, lmax)) = slice_bounds(&layer.values, range) {
            min = min.min(lmin);
            max = max.max(lmax);
        }
    }
    if min.is_finite() && max.is_finite() {
        (min, max)
    } else {
        (0.0, 1.0)
    }
}

fn price_bounds_for_bars(bars: &[crate::asset_data::FinanceOhlcBar]) -> (f64, f64) {
    let mut min = f64::INFINITY;
    let mut max = f64::NEG_INFINITY;
    for bar in bars {
        min = min.min(bar.low);
        max = max.max(bar.high);
    }
    if min.is_finite() && max.is_finite() {
        (min, max)
    } else {
        (0.0, 1.0)
    }
}

fn value_bounds(values: &[f64], range: &Range<usize>) -> (f64, f64) {
    slice_bounds(values, range).unwrap_or((0.0, 1.0))
}

fn slice_bounds(values: &[f64], range: &Range<usize>) -> Option<(f64, f64)> {
    let slice = values.get(range.clone())?;
    let mut min = f64::INFINITY;
    let mut max = f64::NEG_INFINITY;
    for value in slice {
        min = min.min(*value);
        max = max.max(*value);
    }
    if min.is_finite() && max.is_finite() {
        Some((min, max))
    } else {
        None
    }
}

fn draw_grid(pixels: &mut [u8], width: u32, left: u32, top: u32, plot_w: u32, plot_h: u32) {
    for i in 1..4 {
        let y = top + plot_h * i / 4;
        draw_h_line_alpha(pixels, width, left, y, left + plot_w, CHART_GRID_RGB, 180);
    }
}

fn draw_long_shading(
    pixels: &mut [u8],
    width: u32,
    gate: Option<&[f64]>,
    range: &Range<usize>,
    left: u32,
    top: u32,
    plot_w: u32,
    plot_h: u32,
) {
    let Some(gate) = gate else { return };
    let visible_len = range.end.saturating_sub(range.start).max(1);
    let slot_w = plot_w as f64 / visible_len as f64;
    for (index, value) in gate.get(range.clone()).unwrap_or(&[]).iter().enumerate() {
        if *value <= 0.25 {
            continue;
        }
        let x0 = left + (index as f64 * slot_w).floor() as u32;
        let x1 = left + ((index + 1) as f64 * slot_w).ceil() as u32;
        fill_rect_alpha(
            pixels,
            width,
            x0,
            top,
            x1.min(left + plot_w),
            top + plot_h,
            CHART_LONG_SHADE_RGB,
            28,
        );
    }
}

fn draw_candles(
    pixels: &mut [u8],
    width: u32,
    bars: &[crate::asset_data::FinanceOhlcBar],
    left: u32,
    top: u32,
    plot_w: u32,
    plot_h: u32,
) {
    if bars.is_empty() {
        return;
    }
    let (min, max) = price_bounds_for_bars(bars);
    let span = (max - min).max(f64::EPSILON);
    let count = bars.len().max(1) as f64;
    let slot_w = plot_w as f64 / count;
    let body_w = (slot_w * 0.55).max(2.0) as u32;

    for (index, bar) in bars.iter().enumerate() {
        let x_center = left as f64 + slot_w * index as f64 + slot_w * 0.5;
        let y = |price: f64| top as f64 + (max - price) / span * plot_h as f64;
        let bullish = bar.close >= bar.open;
        let rgb = if bullish { CHART_BULL_RGB } else { CHART_BEAR_RGB };
        let y_high = y(bar.high);
        let y_low = y(bar.low);
        draw_v_line(
            pixels,
            width,
            x_center.round() as u32,
            y_high.min(y_low) as u32,
            y_high.max(y_low) as u32,
            rgb,
        );
        let body_top = y(bar.open).min(y(bar.close)) as u32;
        let body_bottom = y(bar.open).max(y(bar.close)) as u32;
        fill_rect(
            pixels,
            width,
            (x_center - body_w as f64 * 0.5).max(left as f64) as u32,
            body_top,
            (x_center + body_w as f64 * 0.5).ceil() as u32,
            body_bottom.max(body_top + 1),
            rgb,
        );
    }
}

fn draw_volume_bars(
    pixels: &mut [u8],
    width: u32,
    bars: &[crate::asset_data::FinanceOhlcBar],
    left: u32,
    top: u32,
    plot_w: u32,
    plot_h: u32,
) {
    if bars.is_empty() {
        return;
    }
    let mut volumes: Vec<f64> = bars.iter().map(|bar| (bar.high - bar.low).max(0.0)).collect();
    let max_vol = volumes.iter().copied().fold(0.0_f64, f64::max).max(f64::EPSILON);
    let count = bars.len().max(1) as f64;
    let slot_w = plot_w as f64 / count;
    for (index, volume) in volumes.iter_mut().enumerate() {
        let bar_h = (*volume / max_vol * plot_h as f64 * 0.85).max(1.0);
        let x0 = left as f64 + slot_w * index as f64 + slot_w * 0.15;
        let x1 = left as f64 + slot_w * index as f64 + slot_w * 0.85;
        fill_rect_alpha(
            pixels,
            width,
            x0 as u32,
            (top + plot_h) as u32 - bar_h as u32,
            x1.ceil() as u32,
            top + plot_h,
            CHART_INDICATOR_RGB,
            160,
        );
    }
}

fn draw_volume_from_series(
    pixels: &mut [u8],
    width: u32,
    values: &[f64],
    range: &Range<usize>,
    left: u32,
    top: u32,
    plot_w: u32,
    plot_h: u32,
) {
    let slice: Vec<f64> = values
        .get(range.clone())
        .unwrap_or(&[])
        .iter()
        .map(|value| value.abs())
        .collect();
    if slice.is_empty() {
        return;
    }
    let max_vol = slice.iter().copied().fold(0.0_f64, f64::max).max(f64::EPSILON);
    let count = slice.len().max(1) as f64;
    let slot_w = plot_w as f64 / count;
    for (index, volume) in slice.iter().enumerate() {
        let bar_h = (*volume / max_vol * plot_h as f64 * 0.85).max(1.0);
        let x0 = left as f64 + slot_w * index as f64 + slot_w * 0.15;
        let x1 = left as f64 + slot_w * index as f64 + slot_w * 0.85;
        fill_rect_alpha(
            pixels,
            width,
            x0 as u32,
            (top + plot_h) as u32 - bar_h as u32,
            x1.ceil() as u32,
            top + plot_h,
            CHART_INDICATOR_RGB,
            120,
        );
    }
}

fn draw_series_line(
    pixels: &mut [u8],
    width: u32,
    values: &[f64],
    range: &Range<usize>,
    left: u32,
    top: u32,
    plot_w: u32,
    plot_h: u32,
    rgb: u32,
    (min, max): (f64, f64),
) {
    let slice = values.get(range.clone()).unwrap_or(&[]);
    if slice.len() < 2 {
        return;
    }
    let span = (max - min).max(f64::EPSILON);
    let last = slice.len().saturating_sub(1).max(1) as f64;
    let mut points = Vec::with_capacity(slice.len());
    for (index, value) in slice.iter().enumerate() {
        let x = left as f64 + (index as f64 / last) * plot_w as f64;
        let y = top as f64 + (max - *value) / span * plot_h as f64;
        points.push((x.round() as i32, y.round() as i32));
    }
    for window in points.windows(2) {
        draw_line_thick(
            pixels,
            width,
            window[0].0,
            window[0].1,
            window[1].0,
            window[1].1,
            rgb,
            2,
        );
    }
}

fn draw_gate_pane(
    pixels: &mut [u8],
    width: u32,
    gate: &[f64],
    range: &Range<usize>,
    left: u32,
    top: u32,
    plot_w: u32,
    plot_h: u32,
) {
    let slice = gate.get(range.clone()).unwrap_or(&[]);
    if slice.is_empty() {
        return;
    }
    let band_h = (plot_h / 3).max(1);
    let count = slice.len().max(1) as f64;
    let slot_w = plot_w as f64 / count;
    for (index, value) in slice.iter().enumerate() {
        let rgb = if *value > 0.25 {
            CHART_GATE_LONG_RGB
        } else if *value < -0.25 {
            CHART_GATE_SHORT_RGB
        } else {
            CHART_GATE_FLAT_RGB
        };
        let y = if *value > 0.25 {
            top
        } else if *value < -0.25 {
            top + band_h * 2
        } else {
            top + band_h
        };
        let x0 = left + (index as f64 * slot_w).floor() as u32;
        let x1 = left + ((index + 1) as f64 * slot_w).ceil() as u32;
        fill_rect(pixels, width, x0, y, x1.min(left + plot_w), y + band_h.saturating_sub(1).max(1), rgb);
    }
}

fn draw_crosshair(
    pixels: &mut [u8],
    width: u32,
    bar_index: usize,
    range: &Range<usize>,
    left: u32,
    top: u32,
    plot_w: u32,
    plot_h: u32,
) {
    if bar_index < range.start || bar_index >= range.end {
        return;
    }
    let visible_len = range.end.saturating_sub(range.start).max(1);
    let rel = (bar_index - range.start) as f64 / visible_len as f64;
    let x = left + (rel * plot_w as f64).round() as u32;
    draw_v_line_alpha(pixels, width, x, top, top + plot_h, 0x758696, 200);
}

const CHART_INDICATOR_RGB: u32 = 0x4a9eff;

fn fill_rgb(pixels: &mut [u8], width: u32, height: u32, rgb: u32) {
    let (r, g, b) = unpack_rgb(rgb);
    for y in 0..height {
        for x in 0..width {
            set_pixel(pixels, width, x, y, r, g, b, 255);
        }
    }
}

fn fill_rect(pixels: &mut [u8], width: u32, x0: u32, y0: u32, x1: u32, y1: u32, rgb: u32) {
    let (r, g, b) = unpack_rgb(rgb);
    for y in y0..y1.min(pixels.len() as u32 / (width * 4)) {
        for x in x0..x1.min(width) {
            set_pixel(pixels, width, x, y, r, g, b, 255);
        }
    }
}

fn fill_rect_alpha(pixels: &mut [u8], width: u32, x0: u32, y0: u32, x1: u32, y1: u32, rgb: u32, alpha: u8) {
    let (r, g, b) = unpack_rgb(rgb);
    for y in y0..y1 {
        for x in x0..x1.min(width) {
            blend_pixel(pixels, width, x, y, r, g, b, alpha);
        }
    }
}

fn draw_h_line_alpha(pixels: &mut [u8], width: u32, x0: u32, y: u32, x1: u32, rgb: u32, alpha: u8) {
    let (r, g, b) = unpack_rgb(rgb);
    for x in x0..=x1.min(width.saturating_sub(1)) {
        blend_pixel(pixels, width, x, y, r, g, b, alpha);
    }
}

fn draw_v_line(pixels: &mut [u8], width: u32, x: u32, y0: u32, y1: u32, rgb: u32) {
    let (r, g, b) = unpack_rgb(rgb);
    for y in y0..=y1 {
        set_pixel(pixels, width, x, y, r, g, b, 255);
    }
}

fn draw_v_line_alpha(pixels: &mut [u8], width: u32, x: u32, y0: u32, y1: u32, rgb: u32, alpha: u8) {
    let (r, g, b) = unpack_rgb(rgb);
    for y in y0..=y1 {
        blend_pixel(pixels, width, x, y, r, g, b, alpha);
    }
}

fn draw_line_thick(
    pixels: &mut [u8],
    width: u32,
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    rgb: u32,
    thickness: i32,
) {
    let points = bresenham_line(x0, y0, x1, y1);
    let (r, g, b) = unpack_rgb(rgb);
    for (x, y) in points {
        for dy in -(thickness / 2)..=(thickness / 2) {
            for dx in -(thickness / 2)..=(thickness / 2) {
                let px = x + dx;
                let py = y + dy;
                if px >= 0 && py >= 0 {
                    set_pixel(pixels, width, px as u32, py as u32, r, g, b, 255);
                }
            }
        }
    }
}

fn bresenham_line(x0: i32, y0: i32, x1: i32, y1: i32) -> Vec<(i32, i32)> {
    let mut points = Vec::new();
    let dx = (x1 - x0).abs();
    let dy = -(y1 - y0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;
    let mut x = x0;
    let mut y = y0;
    loop {
        points.push((x, y));
        if x == x1 && y == y1 {
            break;
        }
        let err2 = 2 * err;
        if err2 >= dy {
            err += dy;
            x += sx;
        }
        if err2 <= dx {
            err += dx;
            y += sy;
        }
    }
    points
}

fn set_pixel(pixels: &mut [u8], width: u32, x: u32, y: u32, r: u8, g: u8, b: u8, a: u8) {
    let index = ((y * width + x) * 4) as usize;
    if index + 3 < pixels.len() {
        pixels[index] = r;
        pixels[index + 1] = g;
        pixels[index + 2] = b;
        pixels[index + 3] = a;
    }
}

fn blend_pixel(pixels: &mut [u8], width: u32, x: u32, y: u32, r: u8, g: u8, b: u8, a: u8) {
    let index = ((y * width + x) * 4) as usize;
    if index + 3 >= pixels.len() {
        return;
    }
    let alpha = a as f32 / 255.0;
    let inv = 1.0 - alpha;
    pixels[index] = (pixels[index] as f32 * inv + r as f32 * alpha) as u8;
    pixels[index + 1] = (pixels[index + 1] as f32 * inv + g as f32 * alpha) as u8;
    pixels[index + 2] = (pixels[index + 2] as f32 * inv + b as f32 * alpha) as u8;
    pixels[index + 3] = 255;
}

fn unpack_rgb(rgb: u32) -> (u8, u8, u8) {
    (
        ((rgb >> 16) & 0xff) as u8,
        ((rgb >> 8) & 0xff) as u8,
        (rgb & 0xff) as u8,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chart_model::build_isolated_series_chart;
    use crate::node_series_cache::{FinanceSeriesKind, NodeValueSummary};

    #[test]
    fn rasterizes_line_pane() {
        let values: Vec<f64> = (0..100).map(|i| i as f64).collect();
        let summary = NodeValueSummary {
            min: 0.0,
            max: 99.0,
            last: 99.0,
            long_pct: None,
            flat_pct: None,
            short_pct: None,
        };
        let model = build_isolated_series_chart(
            "test",
            &values,
            FinanceSeriesKind::Wealth,
            0..100,
            50,
            &summary,
        );
        let bitmap = rasterize_finance_chart_pane(&model, 0, 640, 240).expect("bitmap");
        assert_eq!(bitmap.width, 640);
        assert_eq!(bitmap.height, 240);
    }
}
