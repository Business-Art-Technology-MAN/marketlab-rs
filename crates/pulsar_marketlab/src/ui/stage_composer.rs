//! Stage Composer trait bindings for the USD layer tree.

use gpui::*;
use pulsar_marketlab_ui::workspace::{StageComposerPane, StagePrimRow};

use crate::workspace_state::TradingSystemWorkspace;

impl StageComposerPane for TradingSystemWorkspace {
    fn stage_prim_rows(&self, cx: &App) -> Vec<StagePrimRow> {
        let mut rows: Vec<StagePrimRow> = self
            .usd_stage
            .read(cx)
            .stage_prim_rows()
            .unwrap_or_default()
            .into_iter()
            .map(|row| StagePrimRow {
                path: row.path,
                label: row.label,
                depth: row.depth,
                active: row.active,
            })
            .collect();

        let mut analytics: Vec<String> = self
            .market_stage
            .prims
            .keys()
            .filter(|path| path.starts_with("/analytics/"))
            .cloned()
            .collect();
        analytics.sort();
        for path in analytics {
            rows.push(StagePrimRow {
                label: path.trim_start_matches("/analytics/").to_string(),
                active: self.usd_stage.read(cx).prim_active(&path),
                depth: 1,
                path,
            });
        }

        rows
    }

    fn set_prim_active(&mut self, path: &str, active: bool, cx: &mut Context<Self>) {
        self.usd_stage.update(cx, |bridge, cx| {
            bridge.set_prim_active(path, active);
            cx.notify();
        });
        self.invalidate_playhead_evaluation_cache();

        let usd_stage = self.usd_stage.read(cx).clone();
        let path = path.to_string();
        cx.background_executor()
            .spawn(async move {
                let _ = usd_stage.prim_active(&path);
            })
            .detach();

        self.spawn_playhead_evaluation_async(cx);
    }
}
