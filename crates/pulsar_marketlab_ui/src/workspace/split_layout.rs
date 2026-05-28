//! Custom workstation splitters with GPUI drag capture (global mouse tracking).

use gpui::*;
use gpui::prelude::FluentBuilder;
use gpui_component::AxisExt;

/// Which splitter handle is being dragged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplitHandle {
    /// Upper workstation band vs lower render viewport.
    Vertical,
    /// Stage composer vs node canvas.
    StageCanvas,
    /// Node canvas vs param inspector.
    CanvasInspector,
}

/// Persisted flex shares for the nested workstation tree.
#[derive(Clone, Copy, Debug)]
pub struct WorkstationSplitLayout {
    pub upper_share: f32,
    pub stage_share: f32,
    pub inspector_share: f32,
}

impl Default for WorkstationSplitLayout {
    fn default() -> Self {
        Self {
            upper_share: 0.68,
            stage_share: 0.22,
            inspector_share: 0.30,
        }
    }
}

impl WorkstationSplitLayout {
    pub fn clamp(self) -> Self {
        let upper_share = self.upper_share.clamp(0.30, 0.85);
        let mut stage_share = self.stage_share.clamp(0.12, 0.45);
        let mut inspector_share = self.inspector_share.clamp(0.12, 0.45);
        let mut canvas = 1.0 - stage_share - inspector_share;
        if canvas < 0.15 {
            let deficit = 0.15 - canvas;
            stage_share = (stage_share - deficit * 0.5).max(0.12);
            inspector_share = (inspector_share - deficit * 0.5).max(0.12);
            canvas = 1.0 - stage_share - inspector_share;
        }
        let _ = canvas;
        Self {
            upper_share,
            stage_share,
            inspector_share,
        }
    }
}

pub trait SplitLayoutHost: Sized + 'static {
    fn split_layout(&self) -> WorkstationSplitLayout;
    fn split_container_bounds(&self) -> Option<Bounds<Pixels>>;
    fn upper_row_bounds(&self) -> Option<Bounds<Pixels>>;
    fn set_split_container_bounds(&mut self, bounds: Bounds<Pixels>);
    fn set_upper_row_bounds(&mut self, bounds: Bounds<Pixels>);
    fn active_split_drag(&self) -> Option<SplitHandle>;
    fn set_active_split_drag(&mut self, handle: Option<SplitHandle>);
    fn apply_split_drag(&mut self, handle: SplitHandle, position: Point<Pixels>, cx: &mut Context<Self>);
}

struct SplitterDragGhost;

impl Render for SplitterDragGhost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_0()
    }
}

pub fn render_split_handle<H: SplitLayoutHost>(
    view: Entity<H>,
    handle: SplitHandle,
    axis: Axis,
) -> impl IntoElement {
    let cursor = if axis.is_horizontal() {
        CursorStyle::ResizeColumn
    } else {
        CursorStyle::ResizeRow
    };

    let handle_id = match handle {
        SplitHandle::Vertical => 0usize,
        SplitHandle::StageCanvas => 1usize,
        SplitHandle::CanvasInspector => 2usize,
    };

    div()
        .id(("marketlab-splitter", handle_id))
        .flex_shrink_0()
        .when(axis.is_horizontal(), |this| this.w(px(6.0)).h_full())
        .when(axis.is_vertical(), |this| this.h(px(6.0)).w_full())
        .cursor(cursor)
        .bg(rgb(0x27272a))
        .hover(|style| style.bg(rgb(0x3f3f46)))
        .on_mouse_down(
            MouseButton::Left,
            {
                let view = view.clone();
                move |_: &MouseDownEvent, window, cx| {
                    view.update(cx, |host, cx| {
                        host.set_active_split_drag(Some(handle));
                        host.apply_split_drag(handle, window.mouse_position(), cx);
                    });
                    cx.stop_propagation();
                }
            },
        )
        .on_drag(handle, {
            let view = view.clone();
            move |drag_handle, _offset, window, cx| {
                view.update(cx, |host, cx| {
                    host.set_active_split_drag(Some(*drag_handle));
                    host.apply_split_drag(*drag_handle, window.mouse_position(), cx);
                });
                cx.new(|_| SplitterDragGhost)
            }
        })
        .on_drag_move({
            let view = view.clone();
            move |event: &DragMoveEvent<SplitHandle>, window, cx| {
                if event.drag(cx) != &handle {
                    return;
                }
                view.update(cx, |host, cx| {
                    host.apply_split_drag(handle, window.mouse_position(), cx);
                });
            }
        })
        .capture_any_mouse_up({
            let view = view.clone();
            move |_: &MouseUpEvent, _window, cx| {
                view.update(cx, |host, cx| {
                    if host.active_split_drag().is_some() {
                        host.set_active_split_drag(None);
                        cx.notify();
                    }
                });
            }
        })
}
