//! Stage Composer pane: metadata-focused USD hierarchy tree-table.

use gpui::*;
use gpui::prelude::FluentBuilder;
use gpui_component::button::Button;
use gpui_component::checkbox::Checkbox;
use super::context::{WorkspaceContext, install_ui_selection_observer};
use super::layer_control::layer_display_name;
use super::stage_tree_columns::StageTreeColumnHost;
use super::stage_ledger::render_stage_ledger;
use super::topology_dopesheet::TopologyDopesheetHost;
use crate::theme;
use pulsar_marketlab_core::SESSION_LAYER_FILENAME;

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

pub trait StageComposerPane: StageTreeColumnHost + TopologyDopesheetHost + Sized {
    fn stage_ledger_workspace(&self) -> Entity<WorkspaceContext>;
    fn stage_prim_rows(&self, cx: &App) -> Vec<StagePrimRow>;
    fn stage_tree_collapsed_paths(&self) -> &std::collections::HashSet<String>;
    fn toggle_stage_tree_collapsed(&mut self, path: &str, cx: &mut Context<Self>);
    fn select_stage_path(&mut self, path: Option<String>, cx: &mut Context<Self>);
}

/// Host hook fired after sublayer stack CRUD so the graph engine can resweep.
pub trait LayerStackPane: Sized + 'static {
    fn on_layer_stack_changed(&mut self, cx: &mut Context<Self>);
    fn prompt_import_portfolio_layer(&mut self, cx: &mut Context<Self>);
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
const DCC_SELECTED: u32 = theme::NODE_SELECTION_HALO;

/// USD layer stack control tray (Context Tower shelf).
pub fn render_workstation_layer_stack<H: LayerStackPane + 'static>(
    view: Entity<H>,
    workspace: Entity<WorkspaceContext>,
    cx: &App,
) -> impl IntoElement {
    render_usd_layer_stack_tray(view, workspace, cx)
}

fn render_usd_layer_stack_tray<H: LayerStackPane + 'static>(
    view: Entity<H>,
    workspace: Entity<WorkspaceContext>,
    cx: &App,
) -> impl IntoElement {
    let layers = workspace.read(cx).get_ordered_sublayers();
    let edit_target = workspace.read(cx).edit_target_layer().map(str::to_string);

    let mut stack = div()
        .id("stage-layer-stack")
        .flex_col()
        .w_full()
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .px_3()
                .pt_2()
                .pb_1()
                .child(
                    div()
                        .text_size(px(10.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(DCC_TEXT))
                        .child("OpenUSD Layer Stack"),
                )
                .child(
                    div()
                        .flex()
                        .gap_1()
                        .child(
                            Button::new("create-layer")
                                .label("Create Layer")
                                .on_click({
                                    let view = view.clone();
                                    let workspace = workspace.clone();
                                    move |_event, _window, cx| {
                                        let filename = workspace.update(cx, |ctx, _cx| {
                                            ctx.next_user_layer_filename()
                                        });
                                        let result = workspace.update(cx, |ctx, cx| {
                                            ctx.create_workspace_sublayer(&filename, cx)
                                        });
                                        if result.is_ok() {
                                            view.update(cx, |host, cx| {
                                                host.on_layer_stack_changed(cx);
                                            });
                                        }
                                    }
                                }),
                        )
                        .child(
                            Button::new("import-portfolio-layer")
                                .label("Import Portfolio")
                                .on_click({
                                    let view = view.clone();
                                    move |_event, _window, cx| {
                                        view.update(cx, |host, cx| {
                                            host.prompt_import_portfolio_layer(cx);
                                        });
                                    }
                                }),
                        ),
                ),
        );

    if layers.is_empty() {
        stack = stack.child(
            div()
                .px_3()
                .py_2()
                .text_size(px(9.0))
                .text_color(rgb(DCC_TEXT_MUTED))
                .child("No layers in stage stack."),
        );
        return stack;
    }

    let layer_count = layers.len();
    for (index, layer_id) in layers.into_iter().enumerate() {
        let is_edit_target = edit_target.as_deref() == Some(layer_id.as_str());
        let display = layer_display_name(&layer_id);
        let row_bg = if index % 2 == 0 { DCC_ROW_A } else { DCC_ROW_B };
        let workspace_row = workspace.clone();
        let view_row = view.clone();
        let layer_id_for_toggle = layer_id.clone();
        let layer_id_for_target = layer_id.clone();
        let layer_id_for_delete = layer_id.clone();
        let can_move_up = index > 0;
        let can_move_down = index + 1 < layer_count;
        let is_active = workspace.read(cx).is_layer_active(&layer_id);
        let deletable = layer_id != SESSION_LAYER_FILENAME;

        stack = stack.child(
            div()
                .id(("stage-layer-row", index))
                .flex()
                .items_center()
                .gap_1()
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
                    Checkbox::new(("stage-layer-active", index))
                        .checked(is_active)
                        .on_click({
                            let workspace = workspace_row.clone();
                            let layer_id = layer_id_for_toggle.clone();
                            move |checked, _window, cx| {
                                workspace.update(cx, |ctx, cx| {
                                    ctx.set_layer_active_state(&layer_id, *checked, cx);
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
                        .cursor(CursorStyle::PointingHand)
                        .on_mouse_up(MouseButton::Left, {
                            let workspace = workspace_row.clone();
                            let layer_id = layer_id_for_target.clone();
                            move |_event, _window, cx| {
                                workspace.update(cx, |ctx, cx| {
                                    ctx.set_edit_target_layer(Some(layer_id.clone()), cx);
                                });
                            }
                        })
                        .child(if is_edit_target {
                            format!("📌 {display}")
                        } else {
                            display
                        }),
                )
                .when(can_move_up, |row| {
                    row.child(
                        Button::new(("stage-layer-up", index))
                            .label("⏶")
                            .on_click({
                                let view = view_row.clone();
                                let workspace = workspace_row.clone();
                                let from = index;
                                let to = index - 1;
                                move |_event, _window, cx| {
                                    if workspace
                                        .update(cx, |ctx, cx| {
                                            ctx.reorder_workspace_sublayer(from, to, cx)
                                        })
                                        .is_ok()
                                    {
                                        view.update(cx, |host, cx| {
                                            host.on_layer_stack_changed(cx);
                                        });
                                    }
                                }
                            }),
                    )
                })
                .when(can_move_down, |row| {
                    row.child(
                        Button::new(("stage-layer-down", index))
                            .label("⏷")
                            .on_click({
                                let view = view_row.clone();
                                let workspace = workspace_row.clone();
                                let from = index;
                                let to = index + 1;
                                move |_event, _window, cx| {
                                    if workspace
                                        .update(cx, |ctx, cx| {
                                            ctx.reorder_workspace_sublayer(from, to, cx)
                                        })
                                        .is_ok()
                                    {
                                        view.update(cx, |host, cx| {
                                            host.on_layer_stack_changed(cx);
                                        });
                                    }
                                }
                            }),
                    )
                })
                .when(deletable, |row| {
                    row.child(
                        Button::new(("stage-layer-delete", index))
                            .label("✕")
                            .on_click({
                                let view = view_row.clone();
                                let workspace = workspace_row.clone();
                                let layer_id = layer_id_for_delete.clone();
                                move |_event, _window, cx| {
                                    if workspace
                                        .update(cx, |ctx, cx| {
                                            ctx.remove_sublayer_from_workspace(&layer_id, cx)
                                        })
                                        .is_ok()
                                    {
                                        view.update(cx, |host, cx| {
                                            host.on_layer_stack_changed(cx);
                                        });
                                    }
                                }
                            }),
                    )
                }),
        );
    }

    stack
}

/// Stage Composer body for the Context Tower shelf (ledger explorer).
pub fn render_stage_composer_shelf_body<H: StageComposerPane + 'static>(
    host: &H,
    cx: &mut Context<H>,
) -> impl IntoElement {
    let workspace = host.stage_ledger_workspace();
    div()
        .size_full()
        .min_h_0()
        .bg(rgb(DCC_ROW_B))
        .child(render_stage_ledger(workspace, cx))
}
