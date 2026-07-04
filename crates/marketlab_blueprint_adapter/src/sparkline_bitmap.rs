//! CPU rasterization for financial node sparklines on the EventGraph canvas.

use crate::node_series_cache::FinanceSeriesKind;
use crate::FinanceAssetPreview;

pub const FINANCE_SPARKLINE_WIDTH: u32 = 200;
pub const FINANCE_SPARKLINE_HEIGHT: u32 = 44;
pub const FINANCE_SPARKLINE_MAX_POINTS: usize = 160;
pub const FINANCE_ASSET_SPARKLINE_BLOCK_HEIGHT: f32 = 52.0;
pub const FINANCE_NODE_SPARKLINE_BLOCK_HEIGHT: f32 = FINANCE_ASSET_SPARKLINE_BLOCK_HEIGHT;

const PRICE_STROKE_RGB: u32 = 0x38b86a;
const INDICATOR_STROKE_RGB: u32 = 0x4a9eff;
const WEALTH_STROKE_RGB: u32 = 0xa855f7;
const GATE_LONG_RGB: u32 = 0x38b86a;
const GATE_FLAT_RGB: u32 = 0x52525b;
const GATE_SHORT_RGB: u32 = 0xef4444;
const BACKPLATE_RGB: u32 = 0x1b1b1f;
const GRID_RGB: u32 = 0x27272a;

/// RGBA8 pixel buffer for a node sparkline card.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FinanceSparklineBitmap {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

pub fn rasterize_close_sparkline(closes: &[f64]) -> Option<FinanceSparklineBitmap> {
    rasterize_series_sparkline(closes, FinanceSeriesKind::Price)
}

pub fn rasterize_series_sparkline(
    values: &[f64],
    kind: FinanceSeriesKind,
) -> Option<FinanceSparklineBitmap> {
    if values.len() < 2 {
        return None;
    }
    match kind {
        FinanceSeriesKind::Gate => rasterize_gate_sparkline(values),
        _ => rasterize_line_sparkline(values, stroke_rgb_for_kind(kind)),
    }
}

pub fn rasterize_asset_preview_sparkline(preview: &FinanceAssetPreview) -> Option<FinanceSparklineBitmap> {
    rasterize_close_sparkline(&preview.close_series())
}

fn rasterize_line_sparkline(values: &[f64], stroke_rgb: u32) -> Option<FinanceSparklineBitmap> {
    let values = downsample_series(values, FINANCE_SPARKLINE_MAX_POINTS);
    let width = FINANCE_SPARKLINE_WIDTH;
    let height = FINANCE_SPARKLINE_HEIGHT;
    let mut rgba = vec![0u8; (width * height * 4) as usize];
    fill_rgb(&mut rgba, width, height, BACKPLATE_RGB);

    let inset = 2u32;
    let plot_w = width.saturating_sub(inset * 2).max(1);
    let plot_h = height.saturating_sub(inset * 2).max(1);

    for grid_line in 1..4 {
        let y = inset + plot_h * grid_line / 4;
        draw_h_line(&mut rgba, width, inset, y, inset + plot_w, GRID_RGB);
    }

    let min_value = values.iter().copied().fold(f64::INFINITY, f64::min);
    let max_value = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let raw_span = (max_value - min_value).max(1e-6);
    let y_padding = raw_span * 0.08;
    let y_min = min_value - y_padding;
    let y_max = max_value + y_padding;
    let y_span = (y_max - y_min).max(f64::EPSILON);

    let last = values.len().saturating_sub(1).max(1) as f64;
    let mut points = Vec::with_capacity(values.len());
    for (index, value) in values.iter().enumerate() {
        let t = index as f64 / last;
        let x = inset as f64 + t * plot_w as f64;
        let normalized = (*value - y_min) / y_span;
        let y = inset as f64 + plot_h as f64 - normalized * plot_h as f64;
        points.push((x.round() as i32, y.round() as i32));
    }

    for window in points.windows(2) {
        let (x0, y0) = window[0];
        let (x1, y1) = window[1];
        draw_line(&mut rgba, width, height, x0, y0, x1, y1, stroke_rgb);
    }

    Some(FinanceSparklineBitmap {
        width,
        height,
        rgba,
    })
}

fn rasterize_gate_sparkline(values: &[f64]) -> Option<FinanceSparklineBitmap> {
    let values = downsample_series(values, FINANCE_SPARKLINE_MAX_POINTS);
    let width = FINANCE_SPARKLINE_WIDTH;
    let height = FINANCE_SPARKLINE_HEIGHT;
    let mut rgba = vec![0u8; (width * height * 4) as usize];
    fill_rgb(&mut rgba, width, height, BACKPLATE_RGB);

    let inset = 2u32;
    let plot_w = width.saturating_sub(inset * 2).max(1);
    let plot_h = height.saturating_sub(inset * 2).max(1);
    let band_h = (plot_h / 3).max(1);
    let long_y = inset;
    let flat_y = inset + band_h;
    let short_y = inset + band_h * 2;

    let last = values.len().saturating_sub(1).max(1) as f64;
    for (index, value) in values.iter().enumerate() {
        let t = index as f64 / last;
        let x0 = inset + (t * plot_w as f64).floor() as u32;
        let x1 = if index + 1 < values.len() {
            inset + ((index as f64 + 1.0) / last * plot_w as f64).ceil() as u32
        } else {
            inset + plot_w
        };
        let rgb = if *value > 0.25 {
            GATE_LONG_RGB
        } else if *value < -0.25 {
            GATE_SHORT_RGB
        } else {
            GATE_FLAT_RGB
        };
        let y = if *value > 0.25 {
            long_y
        } else if *value < -0.25 {
            short_y
        } else {
            flat_y
        };
        draw_h_line(&mut rgba, width, x0, y, x1.min(inset + plot_w), rgb);
    }

    Some(FinanceSparklineBitmap {
        width,
        height,
        rgba,
    })
}

fn stroke_rgb_for_kind(kind: FinanceSeriesKind) -> u32 {
    match kind {
        FinanceSeriesKind::Price => PRICE_STROKE_RGB,
        FinanceSeriesKind::Indicator => INDICATOR_STROKE_RGB,
        FinanceSeriesKind::Wealth => WEALTH_STROKE_RGB,
        FinanceSeriesKind::Gate => GATE_LONG_RGB,
    }
}

fn downsample_series(values: &[f64], target: usize) -> Vec<f64> {
    if values.len() <= target {
        return values.to_vec();
    }
    let last = values.len() - 1;
    (0..target)
        .map(|i| {
            let index = (i as f64 * last as f64 / (target - 1) as f64).round() as usize;
            values[index]
        })
        .collect()
}

fn fill_rgb(pixels: &mut [u8], width: u32, height: u32, rgb: u32) {
    let (r, g, b) = unpack_rgb(rgb);
    for y in 0..height {
        for x in 0..width {
            set_pixel(pixels, width, x, y, r, g, b, 255);
        }
    }
}

fn draw_h_line(pixels: &mut [u8], width: u32, x0: u32, y: u32, x1: u32, rgb: u32) {
    let (r, g, b) = unpack_rgb(rgb);
    for x in x0..=x1.min(width.saturating_sub(1)) {
        set_pixel(pixels, width, x, y, r, g, b, 255);
    }
}

fn draw_line(
    pixels: &mut [u8],
    width: u32,
    height: u32,
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    rgb: u32,
) {
    let (r, g, b) = unpack_rgb(rgb);
    let mut x = x0;
    let mut y = y0;
    let dx = (x1 - x0).abs();
    let dy = -(y1 - y0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;

    loop {
        if x >= 0 && y >= 0 {
            let (xu, yu) = (x as u32, y as u32);
            if xu < width && yu < height {
                set_pixel(pixels, width, xu, yu, r, g, b, 255);
            }
        }
        if x == x1 && y == y1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x += sx;
        }
        if e2 <= dx {
            err += dx;
            y += sy;
        }
    }
}

fn set_pixel(pixels: &mut [u8], width: u32, x: u32, y: u32, r: u8, g: u8, b: u8, a: u8) {
    let index = ((y * width + x) * 4) as usize;
    if let Some(chunk) = pixels.get_mut(index..index + 4) {
        chunk[0] = b;
        chunk[1] = g;
        chunk[2] = r;
        chunk[3] = a;
    }
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
    use crate::load_finance_asset_preview;

    #[test]
    fn rasterizes_preview_close_series() {
        let preview = load_finance_asset_preview("SPY", None);
        let bitmap = rasterize_asset_preview_sparkline(&preview).expect("sparkline");
        assert_eq!(bitmap.width, FINANCE_SPARKLINE_WIDTH);
        assert_eq!(bitmap.height, FINANCE_SPARKLINE_HEIGHT);
        assert_eq!(
            bitmap.rgba.len(),
            (FINANCE_SPARKLINE_WIDTH * FINANCE_SPARKLINE_HEIGHT * 4) as usize
        );
        assert!(bitmap.rgba.iter().any(|byte| *byte != 0));
    }
}
