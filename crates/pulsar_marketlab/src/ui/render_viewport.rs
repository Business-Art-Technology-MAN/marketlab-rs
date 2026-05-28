//! Render Viewport trait bindings: ledger grid, playhead slider, OHLC chart.

use gpui::*;
use pulsar_marketlab_ui::workspace::{LedgerRow, RenderViewportPane};

use crate::workspace_state::{format_tick_label, MatrixDataRow, TradingSystemWorkspace};

impl RenderViewportPane for TradingSystemWorkspace {
    fn ledger_rows(&self) -> Vec<LedgerRow> {
        self.inspector_data
            .iter()
            .map(|row: &MatrixDataRow| LedgerRow {
                tick: row.tick.clone(),
                asset: row.asset.clone(),
                grade_type: row.grade_type.clone(),
                value: row.multivector_value.clone(),
            })
            .collect()
    }

    fn playhead_current(&self) -> usize {
        self.playhead_current
    }

    fn playhead_total(&self) -> usize {
        self.playhead_total_bars
    }

    fn playhead_time_label(&self) -> String {
        if self.playhead_time.is_finite() && self.playhead_time != 0.0 {
            format!("{:.0}", self.playhead_time)
        } else {
            format_tick_label(self.playhead_current)
        }
    }

    fn playhead_scrubbing(&self) -> bool {
        self.playhead_scrubbing
    }

    fn set_playhead_scrubbing(&mut self, scrubbing: bool) {
        self.playhead_scrubbing = scrubbing;
    }

    fn playhead_slider_bounds(&self) -> Option<Bounds<Pixels>> {
        self.playhead_slider_bounds
    }

    fn set_playhead_slider_bounds(&mut self, bounds: Bounds<Pixels>) {
        self.playhead_slider_bounds = Some(bounds);
    }

    fn set_playhead_from_slider(&mut self, normalized: f32, cx: &mut Context<Self>) {
        if self.playhead_total_bars < 2 {
            return;
        }
        let max_index = self.playhead_total_bars - 1;
        let index = (normalized * max_index as f32).round() as usize;
        self.set_playhead_index(index.min(max_index), cx);
    }

    fn dispatch_playhead_evaluation_async(&mut self, cx: &mut Context<Self>) {
        self.spawn_playhead_evaluation_async(cx);
    }

    fn render_playhead_chart(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        self.render_ohlc_chart_pane(cx)
    }

    fn status_log_lines(&self) -> &[String] {
        &self.pipeline_status_log
    }
}
