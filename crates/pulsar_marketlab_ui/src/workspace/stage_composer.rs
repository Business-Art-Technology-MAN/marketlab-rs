//! Stage Composer pane: metadata-focused USD hierarchy tree-table.

use gpui::*;
use gpui::prelude::FluentBuilder;
use gpui_component::checkbox::Checkbox;
use openusd::sdf::Value;

use super::context::{WorkspaceContext, install_ui_selection_observer};
use super::stage_tree_columns::{
    render_column_splitter, StageTreeColumnHandle, StageTreeColumnHost, StageTreeColumnLayout,
    STAGE_TREE_ACTIVE_WIDTH, STAGE_TREE_CHEVRON_WIDTH, STAGE_TREE_SPLITTER_HIT_WIDTH,
};
use super::stage_ledger::render_stage_ledger;
use crate::theme;

/// One row in the USD layer hierarchy tree-table.
#[derive(Clone, Debug)]
pub struct StagePrimRow {
    pub path: String,
    pub label: String,
    pub depth: usize,
    pub active: bool,
    pub type_class: String,
    pub weight_allocation: String,
    pub strategy_version: String,
    pub has_children: bool,
}

pub trait StageComposerPane: StageTreeColumnHost + Sized {
    fn stage_ledger_workspace(&self) -> Entity<WorkspaceContext>;
    fn stage_prim_rows(&self, cx: &App) -> Vec<StagePrimRow>;
    fn stage_tree_collapsed_paths(&self) -> &std::collections::HashSet<String>;
    fn toggle_stage_tree_collapsed(&mut self, path: &str, cx: &mut Context<Self>);
    fn select_stage_path(&mut self, path: Option<String>, cx: &mut Context<Self>);
}

/// Repaint the workstation whenever the shared USD stage composition entity notifies.
pub fn install_stage_composition_observer<H: StageComposerPane + 'static, W: 'static>(
    usd_stage: &Entity<W>,
    cx: &mut Context<H>,
) {
    cx.observe(usd_stage, |_host, _stage, cx| {
        cx.notify();
    })
    .detach();
}

/// Observe unified path selection so tree highlights stay in sync with the node canvas.
pub fn install_stage_selection_observer<H: 'static>(
    workspace: &Entity<WorkspaceContext>,
    cx: &mut Context<H>,
) {
    install_ui_selection_observer(workspace, cx);
}

// ── DCC chrome palette ────────────────────────────────────────────────────────

const DCC_ROW_A: u32 = theme::ROW_BACKPLATE_A;
const DCC_ROW_B: u32 = theme::ROW_BACKPLATE_B;
const DCC_DIVIDER: u32 = theme::GRID_MAJOR;
const DCC_TEXT: u32 = theme::TEXT_PRIMARY;
const DCC_TEXT_MUTED: u32 = theme::TEXT_SECONDARY;
const DCC_SELECTED: u32 = theme::TREE_ROW_SELECTED;
const DCC_HEADER: u32 = theme::PANE_BACKPLATE;

fn layer_display_name(identifier: &str) -> String {
    std::path::Path::new(identifier)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(identifier)
        .to_string()
}

fn render_layer_stack(workspace: Entity<WorkspaceContext>, cx: &App) -> impl IntoElement {
    let layers = workspace.read(cx).layer_identifiers();
    let edit_target = workspace.read(cx).edit_target_layer().map(str::to_string);

    let mut stack = div()
        .id("stage-layer-stack")
        .flex_col()
        .max_h(px(120.0))
        .overflow_y_scroll();

    if layers.is_empty() {
        stack = stack.child(
            div()
                .px_3()
                .py_2()
                .text_xs()
                .text_color(rgb(DCC_TEXT_MUTED))
                .child("No layers in stage stack."),
        );
        return stack;
    }

    for (index, layer_id) in layers.into_iter().enumerate() {
        let is_edit_target = edit_target.as_deref() == Some(layer_id.as_str());
        let display = layer_display_name(&layer_id);
        let row_bg = if index % 2 == 0 { DCC_ROW_A } else { DCC_ROW_B };
        let workspace = workspace.clone();
        let layer_id_for_toggle = layer_id.clone();

        stack = stack.child(
            div()
                .id(("stage-layer-row", index))
                .flex()
                .items_center()
                .gap_2()
                .px_2()
                .py_1()
                .bg(rgb(if is_edit_target {
                    DCC_SELECTED
                } else {
                    row_bg
                }))
                .border_b_1()
                .border_color(rgb(DCC_DIVIDER))
                .child(
                    Checkbox::new(("stage-layer-edit", index))
                        .checked(is_edit_target)
                        .on_click({
                            let workspace = workspace.clone();
                            move |checked, _window, cx| {
                                let next = if *checked {
                                    Some(layer_id_for_toggle.clone())
                                } else {
                                    None
                                };
                                workspace.update(cx, |ws, cx| {
                                    ws.set_edit_target_layer(next, cx);
                                });
                            }
                        }),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(px(9.0))
                        .font_family("monospace")
                        .text_color(if is_edit_target {
                            rgb(DCC_TEXT)
                        } else {
                            rgb(DCC_TEXT_MUTED)
                        })
                        .child(display),
                )
                .child(
                    div()
                        .text_size(px(8.0))
                        .text_color(rgb(DCC_TEXT_MUTED))
                        .child(if is_edit_target {
                            "edit target"
                        } else {
                            ""
                        }),
                ),
        );
    }

    stack
}

fn filter_visible_rows(
    rows: Vec<StagePrimRow>,
    collapsed: &std::collections::HashSet<String>,
) -> Vec<StagePrimRow> {
    let mut visible = Vec::new();
    let mut hidden_depth: Option<usize> = None;

    for row in rows {
        if let Some(cutoff) = hidden_depth {
            if row.depth <= cutoff {
                hidden_depth = None;
            } else {
                continue;
            }
        }
        if collapsed.contains(&row.path) {
            hidden_depth = Some(row.depth);
        }
        visible.push(row);
    }
    visible
}

pub fn render_stage_composer<H: StageComposerPane + 'static>(
    view: Entity<H>,
    host: &H,
    cx: &mut Context<H>,
) -> impl IntoElement {
    let workspace = host.stage_ledger_workspace();
    let selected_path = workspace.read(cx).selected_path().map(str::to_string);
    let all_rows = host.stage_prim_rows(cx);
    let collapsed = host.stage_tree_collapsed_paths();
    let rows = filter_visible_rows(all_rows, collapsed);
    let columns = host.stage_tree_columns().clamp();
    let scroll_handle = ScrollHandle::new();
    let view_for_header = view.clone();

    let mut grid = div()
        .id("stage-tree-grid")
        .flex_col()
        .flex_1()
        .min_h_0()
        .min_w(px(columns.total_width()))
        .overflow_x_scroll()
        .overflow_y_scroll()
        .track_scroll(&scroll_handle);

    grid = grid.child(render_tree_header(view_for_header, columns));

    if rows.is_empty() {
        grid = grid.child(
            div()
                .px_3()
                .py_4()
                .text_xs()
                .text_color(rgb(DCC_TEXT_MUTED))
                .child("No USD prims loaded."),
        );
    } else {
        for (row_index, row) in rows.into_iter().enumerate() {
            let is_selected = selected_path.as_deref() == Some(row.path.as_str());
            let scroll_anchor = if is_selected {
                Some(ScrollAnchor::for_handle(scroll_handle.clone()))
            } else {
                None
            };
            grid = grid.child(render_tree_row(
                view.clone(),
                workspace.clone(),
                row,
                row_index,
                is_selected,
                collapsed,
                scroll_anchor,
                columns,
            ));
        }
    }

    div()
        .size_full()
        .flex()
        .flex_col()
        .bg(rgb(DCC_ROW_B))
        .child(
            div()
                .flex_shrink_0()
                .px_3()
                .py_2()
                .border_b_1()
                .border_color(rgb(DCC_DIVIDER))
                .text_size(px(10.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(DCC_TEXT_MUTED))
                .child("Layer Stack"),
        )
        .child(render_layer_stack(workspace.clone(), cx))
        .child(
            div()
                .flex_shrink_0()
                .px_3()
                .py_2()
                .border_b_1()
                .border_color(rgb(DCC_DIVIDER))
                .text_size(px(10.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(DCC_TEXT_MUTED))
                .child("Stage Hierarchy"),
        )
        .child(
            div()
                .id("stage-tree-scroll")
                .flex_1()
                .min_h_0()
                .min_w_0()
                .overflow_x_scroll()
                .child(grid),
        )
        .child(
            div()
                .flex_shrink_0()
                .border_t_1()
                .border_color(rgb(DCC_DIVIDER))
                .max_h(px(160.0))
                .min_h_0()
                .child(render_stage_ledger(workspace, cx)),
        )
}

fn render_tree_header<H: StageComposerPane + 'static>(
    view: Entity<H>,
    columns: StageTreeColumnLayout,
) -> impl IntoElement {
    let view_for_bounds = view.clone();
    div()
        .on_children_prepainted({
            move |bounds: Vec<Bounds<Pixels>>, _window: &mut Window, cx: &mut App| {
                if let Some(header_bounds) = bounds.first().copied() {
                    view_for_bounds.update(cx, |host, _cx| {
                        host.set_stage_tree_header_bounds(header_bounds);
                    });
                }
            }
        })
        .id("stage-tree-header")
        .flex()
        .flex_shrink_0()
        .items_center()
        .min_w(px(columns.total_width()))
        .px_2()
        .py_1()
        .bg(rgb(DCC_HEADER))
        .border_b_1()
        .border_color(rgb(DCC_DIVIDER))
        .text_size(px(9.0))
        .font_weight(FontWeight::BOLD)
        .font_family("monospace")
        .text_color(rgb(DCC_TEXT_MUTED))
        .child(div().w(px(STAGE_TREE_CHEVRON_WIDTH)))
        .child(
            div()
                .flex_shrink_0()
                .w(px(columns.path_width))
                .child("Primitive Node Path"),
        )
        .child(render_column_splitter(view.clone(), StageTreeColumnHandle::PathType))
        .child(
            div()
                .flex_shrink_0()
                .w(px(columns.type_width))
                .child("Type Class"),
        )
        .child(render_column_splitter(view.clone(), StageTreeColumnHandle::TypeWeight))
        .child(
            div()
                .flex_shrink_0()
                .w(px(columns.weight_width))
                .child("Weight/Allocation"),
        )
        .child(render_column_splitter(view.clone(), StageTreeColumnHandle::WeightStrategy))
        .child(
            div()
                .flex_shrink_0()
                .w(px(columns.strategy_width))
                .child("Strategy Version"),
        )
        .child(render_column_splitter(view.clone(), StageTreeColumnHandle::StrategyActive))
        .child(
            div()
                .flex_shrink_0()
                .w(px(STAGE_TREE_ACTIVE_WIDTH))
                .child("Active Status"),
        )
}

fn render_tree_row<H: StageComposerPane + 'static>(
    view: Entity<H>,
    workspace: Entity<WorkspaceContext>,
    row: StagePrimRow,
    row_index: usize,
    is_selected: bool,
    collapsed: &std::collections::HashSet<String>,
    scroll_anchor: Option<ScrollAnchor>,
    columns: StageTreeColumnLayout,
) -> impl IntoElement {
    let path = row.path.clone();
    let toggle_path = row.path.clone();
    let has_children = row.has_children;
    let is_collapsed = collapsed.contains(&row.path);
    let indent = px(14.0 * row.depth as f32);
    let backplate = if row_index % 2 == 0 {
        DCC_ROW_A
    } else {
        DCC_ROW_B
    };
    let bg = if is_selected { DCC_SELECTED } else { backplate };

    let chevron = if row.has_children {
        if is_collapsed { "▸" } else { "▾" }
    } else {
        " "
    };

    let view_for_row = view.clone();
    let view_for_chevron = view.clone();

    let mut row_el = div()
        .id(("stage-tree-row", row_index))
        .flex()
        .flex_shrink_0()
        .items_center()
        .min_w(px(columns.total_width()))
        .px_2()
        .py_1()
        .pl(indent + px(8.0))
        .bg(rgb(bg))
        .border_b_1()
        .border_color(rgb(DCC_DIVIDER))
        .cursor_pointer()
        .hover(|style| style.bg(rgb(DCC_SELECTED)))
        .when_some(scroll_anchor, |this, anchor| this.anchor_scroll(Some(anchor)))
        .on_mouse_up(
            MouseButton::Left,
            {
                let path = path.clone();
                move |_event, window, cx| {
                    view_for_row.update(cx, |host, cx| {
                        host.select_stage_path(Some(path.clone()), cx);
                    });
                    let _ = window;
                }
            },
        );

    let mut chevron_el = div()
        .id(("stage-tree-chevron", row_index))
        .w(px(STAGE_TREE_CHEVRON_WIDTH))
        .text_size(px(10.0))
        .text_color(rgb(DCC_TEXT_MUTED))
        .child(chevron);

    if has_children {
        chevron_el = chevron_el
            .cursor_pointer()
            .hover(|style| style.text_color(rgb(DCC_TEXT)))
            .on_mouse_up(
                MouseButton::Left,
                {
                    let view = view_for_chevron.clone();
                    move |_event, _window, cx| {
                        cx.stop_propagation();
                        view.update(cx, |host, cx| {
                            host.toggle_stage_tree_collapsed(&toggle_path, cx);
                        });
                    }
                },
            );
    }

    row_el = row_el
        .child(chevron_el)
        .child(
            div()
                .flex_shrink_0()
                .w(px(columns.path_width))
                .overflow_hidden()
                .flex_col()
                .gap_0p5()
                .child(
                    div()
                        .w_full()
                        .overflow_hidden()
                        .text_size(px(9.0))
                        .font_family("monospace")
                        .text_color(if row.active {
                            rgb(DCC_TEXT)
                        } else {
                            rgb(DCC_TEXT_MUTED)
                        })
                        .child(row.path.clone()),
                )
                .when(!row.label.is_empty() && row.label != row.path, |this| {
                    this.child(
                        div()
                            .w_full()
                            .overflow_hidden()
                            .text_size(px(8.0))
                            .text_color(rgb(DCC_TEXT_MUTED))
                            .child(row.label.clone()),
                    )
                }),
        )
        .child(div().w(px(STAGE_TREE_SPLITTER_HIT_WIDTH)))
        .child(
            div()
                .flex_shrink_0()
                .w(px(columns.type_width))
                .overflow_hidden()
                .text_size(px(9.0))
                .font_family("monospace")
                .text_color(rgb(DCC_TEXT_MUTED))
                .child(row.type_class),
        )
        .child(div().w(px(STAGE_TREE_SPLITTER_HIT_WIDTH)))
        .child(
            div()
                .flex_shrink_0()
                .w(px(columns.weight_width))
                .overflow_hidden()
                .text_size(px(9.0))
                .font_family("monospace")
                .text_color(rgb(DCC_TEXT_MUTED))
                .child(row.weight_allocation),
        )
        .child(div().w(px(STAGE_TREE_SPLITTER_HIT_WIDTH)))
        .child(
            div()
                .flex_shrink_0()
                .w(px(columns.strategy_width))
                .overflow_hidden()
                .text_size(px(9.0))
                .font_family("monospace")
                .text_color(rgb(DCC_TEXT_MUTED))
                .child(row.strategy_version),
        )
        .child(div().w(px(STAGE_TREE_SPLITTER_HIT_WIDTH)))
        .child(
            div()
                .flex_shrink_0()
                .w(px(STAGE_TREE_ACTIVE_WIDTH))
                .child(
                    Checkbox::new(("stage-active", row_index))
                        .checked(row.active)
                        .on_click({
                            let workspace = workspace.clone();
                            let path = path.clone();
                            move |checked, window, cx| {
                                let next_state = *checked;
                                workspace.update(cx, |ws, cx| {
                                    ws.modify_attribute(
                                        &path,
                                        "inputs:active",
                                        Value::Bool(next_state),
                                        cx,
                                    );
                                });
                                let _ = window;
                            }
                        }),
                ),
        );

    row_el
}
