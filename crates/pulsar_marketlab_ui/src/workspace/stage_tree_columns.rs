//! Draggable column splitters for the Stage Hierarchy tree-table.

use gpui::*;

/// Fixed leading gutter for collapse chevrons.
pub const STAGE_TREE_CHEVRON_WIDTH: f32 = 18.0;
/// Fixed trailing column for the active checkbox.
pub const STAGE_TREE_ACTIVE_WIDTH: f32 = 48.0;
/// Minimum width for resizable metadata columns.
pub const STAGE_TREE_COLUMN_MIN: f32 = 48.0;
/// Minimum width for the path column (chevrons must stay clickable).
pub const STAGE_TREE_PATH_MIN: f32 = 96.0;
/// Hit target width for column boundary drag handles.
pub const STAGE_TREE_SPLITTER_HIT_WIDTH: f32 = 6.0;

/// Persisted pixel widths for the stage hierarchy columns.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StageTreeColumnLayout {
    pub path_width: f32,
    pub type_width: f32,
    pub weight_width: f32,
    pub strategy_width: f32,
}

impl Default for StageTreeColumnLayout {
    fn default() -> Self {
        Self {
            path_width: 240.0,
            type_width: 88.0,
            weight_width: 72.0,
            strategy_width: 88.0,
        }
    }
}

impl StageTreeColumnLayout {
    pub fn clamp(self) -> Self {
        Self {
            path_width: self.path_width.max(STAGE_TREE_PATH_MIN),
            type_width: self.type_width.max(STAGE_TREE_COLUMN_MIN),
            weight_width: self.weight_width.max(STAGE_TREE_COLUMN_MIN),
            strategy_width: self.strategy_width.max(STAGE_TREE_COLUMN_MIN),
        }
    }

    pub fn total_width(&self) -> f32 {
        STAGE_TREE_CHEVRON_WIDTH
            + self.path_width
            + self.type_width
            + self.weight_width
            + self.strategy_width
            + STAGE_TREE_ACTIVE_WIDTH
            + STAGE_TREE_SPLITTER_HIT_WIDTH * 4.0
    }

    pub fn apply_drag(&mut self, handle: StageTreeColumnHandle, delta_x: f32) {
        match handle {
            StageTreeColumnHandle::PathType => {
                self.path_width = (self.path_width + delta_x).max(STAGE_TREE_PATH_MIN);
            }
            StageTreeColumnHandle::TypeWeight => {
                self.type_width = (self.type_width + delta_x).max(STAGE_TREE_COLUMN_MIN);
            }
            StageTreeColumnHandle::WeightStrategy => {
                self.weight_width = (self.weight_width + delta_x).max(STAGE_TREE_COLUMN_MIN);
            }
            StageTreeColumnHandle::StrategyActive => {
                self.strategy_width = (self.strategy_width + delta_x).max(STAGE_TREE_COLUMN_MIN);
            }
        }
    }
}

/// Boundary between two resizable stage tree columns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StageTreeColumnHandle {
    PathType,
    TypeWeight,
    WeightStrategy,
    StrategyActive,
}

pub trait StageTreeColumnHost: Sized + 'static {
    fn stage_tree_columns(&self) -> StageTreeColumnLayout;
    fn set_stage_tree_columns(&mut self, layout: StageTreeColumnLayout);
    fn stage_tree_header_bounds(&self) -> Option<Bounds<Pixels>>;
    fn set_stage_tree_header_bounds(&mut self, bounds: Bounds<Pixels>);
    fn active_stage_tree_column_drag(&self) -> Option<(StageTreeColumnHandle, f32)>;
    fn set_active_stage_tree_column_drag(&mut self, drag: Option<(StageTreeColumnHandle, f32)>);
}

struct ColumnSplitterDragGhost;

fn splitter_id(handle: StageTreeColumnHandle) -> usize {
    match handle {
        StageTreeColumnHandle::PathType => 0,
        StageTreeColumnHandle::TypeWeight => 1,
        StageTreeColumnHandle::WeightStrategy => 2,
        StageTreeColumnHandle::StrategyActive => 3,
    }
}

impl Render for ColumnSplitterDragGhost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_0()
    }
}

pub fn render_column_splitter<H: StageTreeColumnHost>(
    view: Entity<H>,
    handle: StageTreeColumnHandle,
) -> impl IntoElement {
    div()
        .id(("stage-tree-splitter", splitter_id(handle)))
        .flex_shrink_0()
        .w(px(STAGE_TREE_SPLITTER_HIT_WIDTH))
        .h_full()
        .cursor(CursorStyle::ResizeColumn)
        .hover(|style| style.bg(rgb(0x3f3f46)))
        .on_mouse_down(
            MouseButton::Left,
            {
                let view = view.clone();
                move |event: &MouseDownEvent, _window, cx| {
                    let start_x: f32 = event.position.x.into();
                    view.update(cx, |host, _cx| {
                        host.set_active_stage_tree_column_drag(Some((handle, start_x)));
                    });
                    cx.stop_propagation();
                }
            },
        )
        .on_drag(handle, {
            let view = view.clone();
            move |drag_handle, _offset, _window, cx| {
                view.update(cx, |host, _cx| {
                    host.set_active_stage_tree_column_drag(Some((*drag_handle, 0.0)));
                });
                cx.new(|_| ColumnSplitterDragGhost)
            }
        })
        .on_drag_move({
            let view = view.clone();
            move |event: &DragMoveEvent<StageTreeColumnHandle>, _window, cx| {
                if event.drag(cx) != &handle {
                    return;
                }
                let current_x: f32 = event.event.position.x.into();
                view.update(cx, |host, cx| {
                    if let Some((active, start_x)) = host.active_stage_tree_column_drag() {
                        if active != handle {
                            return;
                        }
                        let delta = current_x - start_x;
                        if delta.abs() > f32::EPSILON {
                            let mut columns = host.stage_tree_columns();
                            columns.apply_drag(handle, delta);
                            host.set_stage_tree_columns(columns.clamp());
                            host.set_active_stage_tree_column_drag(Some((handle, current_x)));
                            cx.notify();
                        }
                    }
                });
            }
        })
        .capture_any_mouse_up({
            let view = view.clone();
            move |_: &MouseUpEvent, _window, cx| {
                view.update(cx, |host, cx| {
                    if host.active_stage_tree_column_drag().is_some() {
                        host.set_active_stage_tree_column_drag(None);
                        cx.notify();
                    }
                });
            }
        })
}
