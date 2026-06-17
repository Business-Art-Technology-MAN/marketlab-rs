//! One-time CPU rasterization for asset node sparklines (blitted each frame via `img()`).

use std::collections::HashMap;
use std::sync::Arc;

use gpui::RenderImage;
use image::{ImageBuffer, RgbaImage};

use crate::workspace_state::{
    parse_chart_date_ordinal, ChartHistoryBuffer, CHART_Y_MIN_SPAN, CHART_Y_PADDING_RATIO,
};

pub const ASSET_CHART_BITMAP_WIDTH: u32 = 200;
pub const ASSET_CHART_BITMAP_HEIGHT: u32 = 48;
pub const ASSET_CHART_BITMAP_MAX_POINTS: usize = 320;
pub const ASSET_CHART_STROKE_RGB: u32 = 0xd4a054;
const ASSET_CHART_BACKPLATE_RGB: u32 = 0x1b1b1f;
const GRID_RGB: u32 = 0x27272a;

pub fn build_asset_chart_bitmaps(
    history: &HashMap<usize, ChartHistoryBuffer>,
) -> HashMap<usize, Arc<RenderImage>> {
    history
        .iter()
        .filter_map(|(node_id, buffer)| {
            rasterize_asset_chart_bitmap(buffer, ASSET_CHART_STROKE_RGB)
                .map(|image| (*node_id, image))
        })
        .collect()
}

pub fn rasterize_asset_chart_bitmap(
    buffer: &ChartHistoryBuffer,
    stroke_rgb: u32,
) -> Option<Arc<RenderImage>> {
    if buffer.values.len() < 2 || buffer.timestamps.len() != buffer.values.len() {
        return None;
    }

    let (x_coords, values) = downsample_chart_series(buffer)?;
    let width = ASSET_CHART_BITMAP_WIDTH;
    let height = ASSET_CHART_BITMAP_HEIGHT;
    let mut pixels = vec![0u8; (width * height * 4) as usize];
    fill_rgb(&mut pixels, width, height, ASSET_CHART_BACKPLATE_RGB);

    let inset = 2u32;
    let plot_w = width.saturating_sub(inset * 2).max(1);
    let plot_h = height.saturating_sub(inset * 2).max(1);

    for grid_line in 1..4 {
        let y = inset + plot_h * grid_line / 4;
        draw_h_line(&mut pixels, width, inset, y, inset + plot_w, GRID_RGB);
    }

    let min_value = values.iter().copied().fold(f32::INFINITY, f32::min);
    let max_value = values.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let raw_span = (max_value - min_value).max(CHART_Y_MIN_SPAN);
    let y_padding = raw_span * CHART_Y_PADDING_RATIO;
    let y_min = min_value - y_padding;
    let y_max = max_value + y_padding;
    let y_span = (y_max - y_min).max(f32::EPSILON);

    let x_min = x_coords[0];
    let x_max = x_coords[x_coords.len() - 1];
    let x_span = (x_max - x_min).max(f32::EPSILON);

    let mut points = Vec::with_capacity(values.len());
    for (index, value) in values.iter().enumerate() {
        let t = (x_coords[index] - x_min) / x_span;
        let x = inset as f32 + t * plot_w as f32;
        let normalized = (*value - y_min) / y_span;
        let y = inset as f32 + plot_h as f32 - normalized * plot_h as f32;
        points.push((x.round() as i32, y.round() as i32));
    }

    for window in points.windows(2) {
        let (x0, y0) = window[0];
        let (x1, y1) = window[1];
        draw_line(&mut pixels, width, height, x0, y0, x1, y1, stroke_rgb);
    }

    let image: RgbaImage = ImageBuffer::from_raw(width, height, pixels)?;
    Some(Arc::new(RenderImage::new(vec![image::Frame::new(image)])))
}

fn downsample_chart_series(buffer: &ChartHistoryBuffer) -> Option<(Vec<f32>, Vec<f32>)> {
    let mut x_coords = Vec::with_capacity(buffer.values.len());
    let mut values = Vec::with_capacity(buffer.values.len());
    for (date, value) in buffer.timestamps.iter().zip(buffer.values.iter()) {
        let x = parse_chart_date_ordinal(date)?;
        x_coords.push(x);
        values.push(*value);
    }
    if x_coords.len() < 2 {
        return None;
    }
    if x_coords.len() <= ASSET_CHART_BITMAP_MAX_POINTS {
        return Some((x_coords, values));
    }

    let target = ASSET_CHART_BITMAP_MAX_POINTS;
    let last = x_coords.len() - 1;
    let mut sampled_x = Vec::with_capacity(target);
    let mut sampled_v = Vec::with_capacity(target);
    for i in 0..target {
        let index = (i as f64 * last as f64 / (target - 1) as f64).round() as usize;
        sampled_x.push(x_coords[index]);
        sampled_v.push(values[index]);
    }
    Some((sampled_x, sampled_v))
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
                if xu + 1 < width {
                    set_pixel(pixels, width, xu + 1, yu, r, g, b, 255);
                }
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
