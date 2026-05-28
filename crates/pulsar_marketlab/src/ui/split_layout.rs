//! Split layout trait bindings for workstation resize handles.

use gpui::*;
use pulsar_marketlab_ui::workspace::{SplitHandle, SplitLayoutHost, WorkstationSplitLayout};

use crate::workspace_state::TradingSystemWorkspace;

impl SplitLayoutHost for TradingSystemWorkspace {
    fn split_layout(&self) -> WorkstationSplitLayout {
        self.split_layout
    }

    fn split_container_bounds(&self) -> Option<Bounds<Pixels>> {
        self.split_container_bounds
    }

    fn upper_row_bounds(&self) -> Option<Bounds<Pixels>> {
        self.upper_row_bounds
    }

    fn set_split_container_bounds(&mut self, bounds: Bounds<Pixels>) {
        self.split_container_bounds = Some(bounds);
    }

    fn set_upper_row_bounds(&mut self, bounds: Bounds<Pixels>) {
        self.upper_row_bounds = Some(bounds);
    }

    fn active_split_drag(&self) -> Option<SplitHandle> {
        self.active_split_drag
    }

    fn set_active_split_drag(&mut self, handle: Option<SplitHandle>) {
        self.active_split_drag = handle;
    }

    fn apply_split_drag(&mut self, handle: SplitHandle, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(container) = self.split_container_bounds else {
            return;
        };
        let mut layout = self.split_layout.clamp();
        match handle {
            SplitHandle::Vertical => {
                let height: f32 = container.size.height.into();
                if height <= f32::EPSILON {
                    return;
                }
                let y: f32 = position.y.into();
                let origin_y: f32 = container.origin.y.into();
                layout.upper_share = ((y - origin_y) / height).clamp(0.30, 0.85);
            }
            SplitHandle::StageCanvas | SplitHandle::CanvasInspector => {
                let Some(upper) = self.upper_row_bounds else {
                    return;
                };
                let width: f32 = upper.size.width.into();
                if width <= f32::EPSILON {
                    return;
                }
                let x: f32 = position.x.into();
                let origin_x: f32 = upper.origin.x.into();
                let normalized = ((x - origin_x) / width).clamp(0.12, 0.88);
                match handle {
                    SplitHandle::StageCanvas => {
                        layout.stage_share = normalized;
                        layout.inspector_share = (layout.inspector_share)
                            .min(1.0 - layout.stage_share - 0.15);
                    }
                    SplitHandle::CanvasInspector => {
                        layout.inspector_share = (1.0 - normalized).clamp(0.12, 0.45);
                        layout.stage_share = layout.stage_share.min(1.0 - layout.inspector_share - 0.15);
                    }
                    SplitHandle::Vertical => {}
                }
            }
        }
        self.split_layout = layout.clamp();
        cx.notify();
    }
}
