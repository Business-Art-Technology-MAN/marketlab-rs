//! Bottom topology panel: USD layer stack + logical strategy hierarchy (no bar matrix).

use std::hash::{Hash, Hasher};

use gpui::*;
use gpui::prelude::FluentBuilder;
use gpui_component::checkbox::Checkbox;
use gpui_component::scroll::ScrollableElement;

use super::context::WorkspaceContext;
use super::dopesheet_state::DopesheetUiState;
use super::layer_control::{LayerDescriptor, LayerDisplayState};
use super::logical_topology::LogicalTreeNode;
use crate::theme;

const ROW_A: u32 = theme::ROW_BACKPLATE_A;
const ROW_B: u32 = theme::ROW_BACKPLATE_B;
const DIVIDER: u32 = theme::GRID_MAJOR;
const TEXT: u32 = theme::TEXT_PRIMARY;
const MUTED: u32 = theme::TEXT_SECONDARY;
const LAYER_ACTIVE: u32 = 0x2D_37_48;
const LAYER_MUTED: u32 = 0x1A_1A_1A;
const LAYER_ISOLATED: u32 = 0x2B_6C_B0;
const LAYER_OPINION: u32 = 0x66_CC_FF;
const MAX_DOPESHEET_ROWS: usize = 256;
const DISCLOSURE_WIDTH: f32 = 12.0;
const DEPTH_INDENT: f32 = 14.0;
const ROW_HEIGHT: f32 = 22.0;
const DISCLOSURE_COLLAPSED: &str = "▸";
const DISCLOSURE_EXPANDED: &str = "▾";

pub trait TopologyDopesheetHost: Sized + 'static {
    fn topology_workspace(&self) -> Entity<WorkspaceContext>;
    fn topology_dopesheet_ui_state(&self) -> &DopesheetUiState;
    fn topology_dopesheet_vertical_scroll_handle(&self) -> &ScrollHandle;
    fn topology_ensure_strategy_tree(&mut self, cx: &mut Context<Self>);
    fn topology_strategy_tree(&self) -> &[LogicalTreeNode];
    fn topology_select_prim(&mut self, path: Option<String>, cx: &mut Context<Self>);
    fn topology_toggle_node_expansion(&mut self, prim_path: &str, cx: &mut Context<Self>);
    fn topology_on_prim_active_changed(&mut self, cx: &mut Context<Self>);
    fn topology_on_layer_state_changed(&mut self, cx: &mut Context<Self>);
}

/// Full-width bottom anchor: layer composition header + scrollable strategy hierarchy.
pub fn render_unified_bottom_topology_dopesheet<H: TopologyDopesheetHost>(
    view: Entity<H>,
    host: &mut H,
    cx: &mut Context<H>,
) -> impl IntoElement {
    render_topology_dopesheet_panel(view, host, cx)
}

pub fn render_topology_dopesheet_panel<H: TopologyDopesheetHost>(
    view: Entity<H>,
    host: &mut H,
    cx: &mut Context<H>,
) -> impl IntoElement {
    host.topology_ensure_strategy_tree(cx);
    let workspace = host.topology_workspace();
    let ui_state = host.topology_dopesheet_ui_state();
    let strategy_tree = host.topology_strategy_tree();
    let layer_stack = workspace.read(cx).get_active_layer_stack_descriptors();
    let vertical_scroll = host.topology_dopesheet_vertical_scroll_handle().clone();
    let loose_otl_sandbox = workspace.read(cx).is_loose_otl_sandbox_mode();

    div()
        .id("topology-dopesheet-panel")
        .size_full()
        .min_h_0()
        .flex()
        .flex_col()
        .bg(rgb(ROW_A))
        .child(render_layer_stack_manager_header(view.clone(), workspace.clone(), &layer_stack))
        .child(
            div()
                .flex_shrink_0()
                .h(px(1.0))
                .bg(rgb(DIVIDER)),
        )
        .child(
            div()
                .flex_shrink_0()
                .px_3()
                .py_1()
                .border_b_1()
                .border_color(rgb(DIVIDER))
                .bg(rgb(ROW_B))
                .text_size(px(9.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(MUTED))
                .child("Logical Strategy Hierarchy"),
        )
        .child(
            div()
                .flex_1()
                .min_h_0()
                .vertical_scrollbar(&vertical_scroll)
                .child(render_strategy_hierarchy_tree(
                    view,
                    workspace,
                    ui_state,
                    cx,
                    strategy_tree,
                    loose_otl_sandbox,
                )),
        )
}

fn render_layer_stack_manager_header<H: TopologyDopesheetHost + 'static>(
    view: Entity<H>,
    workspace: Entity<WorkspaceContext>,
    layers: &[LayerDescriptor],
) -> impl IntoElement {
    div()
        .flex_shrink_0()
        .flex_col()
        .gap_2()
        .px_3()
        .py_2()
        .child(
            div()
                .text_size(px(10.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(TEXT))
                .child("Layer Composition Plane"),
        )
        .child(
            div()
                .flex()
                .flex_wrap()
                .gap_2()
                .items_center()
                .child(
                    div()
                        .text_size(px(9.0))
                        .text_color(rgb(MUTED))
                        .child("Opinion strength:"),
                )
                .children(layers.iter().enumerate().map(|(index, layer)| {
                    let layer_filename = layer.filename.clone();
                    let layer_display = layer.display_name.clone();
                    let bg = match layer.state {
                        LayerDisplayState::Active => rgb(LAYER_ACTIVE),
                        LayerDisplayState::Muted => rgb(LAYER_MUTED),
                        LayerDisplayState::Isolated => rgb(LAYER_ISOLATED),
                    };
                    let view = view.clone();
                    let workspace = workspace.clone();
                    let can_move_up = index > 0;
                    let can_move_down = index + 1 < layers.len();
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .child(
                            div()
                                .id(("layer-pill", index))
                                .px_2()
                                .py_1()
                                .rounded_md()
                                .bg(bg)
                                .border_1()
                                .border_color(rgb(DIVIDER))
                                .text_size(px(9.0))
                                .font_family("monospace")
                                .text_color(rgb(TEXT))
                                .cursor(CursorStyle::PointingHand)
                                .on_mouse_up(MouseButton::Left, {
                                    let layer_filename = layer_filename.clone();
                                    let view = view.clone();
                                    let workspace = workspace.clone();
                                    move |_event, _window, cx| {
                                        workspace.update(cx, |ctx, cx| {
                                            ctx.set_selected_active_target_layer(
                                                &layer_filename,
                                                cx,
                                            );
                                        });
                                        view.update(cx, |host, cx| {
                                            host.topology_on_layer_state_changed(cx);
                                        });
                                        cx.stop_propagation();
                                    }
                                })
                                .on_mouse_up(MouseButton::Right, {
                                    let layer_filename = layer_filename.clone();
                                    let view = view.clone();
                                    let workspace = workspace.clone();
                                    move |_event, _window, cx| {
                                        workspace.update(cx, |ctx, cx| {
                                            ctx.toggle_layer_mute(&layer_filename, cx);
                                        });
                                        view.update(cx, |host, cx| {
                                            host.topology_on_layer_state_changed(cx);
                                        });
                                        cx.stop_propagation();
                                    }
                                })
                                .child(format!("{layer_display}")),
                        )
                        .when(can_move_up, |row| {
                            let view = view.clone();
                            let workspace = workspace.clone();
                            row.child(
                                div()
                                    .text_size(px(8.0))
                                    .text_color(rgb(MUTED))
                                    .cursor(CursorStyle::PointingHand)
                                    .on_mouse_up(MouseButton::Left, move |_event, _window, cx| {
                                        workspace.update(cx, |ctx, cx| {
                                            ctx.reorder_layer(index, index - 1, cx);
                                        });
                                        view.update(cx, |host, cx| {
                                            host.topology_on_layer_state_changed(cx);
                                        });
                                        cx.stop_propagation();
                                    })
                                    .child("▲"),
                            )
                        })
                        .when(can_move_down, |row| {
                            let view = view.clone();
                            let workspace = workspace.clone();
                            row.child(
                                div()
                                    .text_size(px(8.0))
                                    .text_color(rgb(MUTED))
                                    .cursor(CursorStyle::PointingHand)
                                    .on_mouse_up(MouseButton::Left, move |_event, _window, cx| {
                                        workspace.update(cx, |ctx, cx| {
                                            ctx.reorder_layer(index, index + 1, cx);
                                        });
                                        view.update(cx, |host, cx| {
                                            host.topology_on_layer_state_changed(cx);
                                        });
                                        cx.stop_propagation();
                                    })
                                    .child("▼"),
                            )
                        })
                        .when(index + 1 < layers.len(), |row| {
                            row.child(
                                div()
                                    .text_size(px(8.0))
                                    .text_color(rgb(MUTED))
                                    .child("→"),
                            )
                        })
                })),
        )
}

fn dopesheet_empty_message(loose_otl_sandbox: bool) -> &'static str {
    if loose_otl_sandbox {
        "OTL sandbox — add assets or OTL scripts to the canvas, or import a portfolio layer."
    } else {
        "No strategy topology compiled — wire portfolio integrators or load assets."
    }
}

fn render_strategy_hierarchy_tree<H: TopologyDopesheetHost + 'static>(
    view: Entity<H>,
    workspace: Entity<WorkspaceContext>,
    ui_state: &DopesheetUiState,
    cx: &App,
    strategy_tree: &[LogicalTreeNode],
    loose_otl_sandbox: bool,
) -> impl IntoElement {
    if strategy_tree.is_empty() {
        return div()
            .px_3()
            .py_4()
            .text_size(px(10.0))
            .text_color(rgb(MUTED))
            .child(dopesheet_empty_message(loose_otl_sandbox))
            .into_any_element();
    }

    let mut stack = div().id("dopesheet-hierarchy-rows").flex_col().min_w_0();
    let mut row_index = 0usize;
    for root in strategy_tree {
        stack = append_hierarchy_row_recursive(
            stack,
            view.clone(),
            workspace.clone(),
            ui_state,
            cx,
            root,
            0,
            &mut row_index,
        );
        if row_index >= MAX_DOPESHEET_ROWS {
            stack = stack.child(
                div()
                    .px_3()
                    .py_2()
                    .text_size(px(9.0))
                    .text_color(rgb(MUTED))
                    .child("… truncated"),
            );
            break;
        }
    }
    stack.into_any_element()
}

fn append_hierarchy_row_recursive<H: TopologyDopesheetHost + 'static>(
    mut hierarchy_stack: Stateful<Div>,
    view: Entity<H>,
    workspace: Entity<WorkspaceContext>,
    ui_state: &DopesheetUiState,
    cx: &App,
    node: &LogicalTreeNode,
    depth: usize,
    row_index: &mut usize,
) -> Stateful<Div> {
    if *row_index >= MAX_DOPESHEET_ROWS {
        return hierarchy_stack;
    }
    *row_index += 1;
    let prim_path = node.prim_path.clone();
    let ws = workspace.read(cx);
    let is_selected = ws.is_selected(&prim_path);
    let is_expanded = ui_state.is_expanded(&prim_path, depth);
    let has_children = !node.children.is_empty();
    let is_active = ws.is_node_enabled_in_stage(&prim_path);
    let dominating_layer = ws.get_dominating_layer_for_prim(&prim_path);
    let row_bg = if depth % 2 == 0 { ROW_A } else { ROW_B };

    let mut topology_col = div()
        .w_full()
        .flex()
        .items_center()
        .gap_1()
        .pl(px(depth as f32 * DEPTH_INDENT))
        .pr_1();

    if has_children {
        let path_for_expand = prim_path.clone();
        let chevron = if is_expanded {
            DISCLOSURE_EXPANDED
        } else {
            DISCLOSURE_COLLAPSED
        };
        topology_col = topology_col.child(
            div()
                .id(("topology-disclosure", stable_prim_element_key(&path_for_expand)))
                .w(px(DISCLOSURE_WIDTH))
                .flex_shrink_0()
                .text_size(px(10.0))
                .text_color(rgb(MUTED))
                .cursor(CursorStyle::PointingHand)
                .hover(|style| style.text_color(rgb(TEXT)))
                .on_mouse_down(MouseButton::Left, {
                    let view = view.clone();
                    let path_for_expand = path_for_expand.clone();
                    move |_event, _window, cx| {
                        view.update(cx, |host, cx| {
                            host.topology_toggle_node_expansion(&path_for_expand, cx);
                        });
                        cx.stop_propagation();
                    }
                })
                .child(chevron),
        );
    } else {
        topology_col = topology_col.child(div().w(px(DISCLOSURE_WIDTH)).flex_shrink_0());
    }

    let path_for_select = prim_path.clone();
    let is_branch = has_children;
    topology_col = topology_col.child(
        div()
            .flex_1()
            .min_w_0()
            .text_size(px(10.0))
            .font_weight(if is_selected || is_branch {
                FontWeight::SEMIBOLD
            } else {
                FontWeight::NORMAL
            })
            .text_color(if is_selected {
                rgb(TEXT)
            } else if is_branch {
                rgb(TEXT)
            } else {
                rgb(MUTED)
            })
            .truncate()
            .cursor(CursorStyle::PointingHand)
            .on_mouse_up(MouseButton::Left, {
                let view = view.clone();
                let path_for_select = path_for_select.clone();
                move |_event, _window, cx| {
                    view.update(cx, |host, cx| {
                        host.topology_select_prim(Some(path_for_select.clone()), cx);
                    });
                    cx.stop_propagation();
                }
            })
            .child(node.display_label.clone()),
    );

    let path_for_toggle = prim_path.clone();
    topology_col = topology_col.child(
        Checkbox::new(("topology-active", stable_prim_element_key(&path_for_toggle)))
            .checked(is_active)
            .on_click({
                let view = view.clone();
                let workspace = workspace.clone();
                let path_for_toggle = path_for_toggle.clone();
                move |checked, _window, cx| {
                    workspace.update(cx, |ctx, cx| {
                        ctx.set_node_enabled_in_stage(&path_for_toggle, *checked, cx);
                    });
                    view.update(cx, |host, cx| {
                        host.topology_on_prim_active_changed(cx);
                    });
                }
            }),
    );

    if let Some(layer_name) = dominating_layer {
        topology_col = topology_col.child(
            div()
                .flex_shrink_0()
                .text_size(px(8.0))
                .font_family("monospace")
                .text_color(rgb(LAYER_OPINION))
                .child(layer_name),
        );
    }

    hierarchy_stack = hierarchy_stack.child(
        div()
            .flex()
            .flex_shrink_0()
            .items_center()
            .h(px(ROW_HEIGHT))
            .min_h(px(ROW_HEIGHT))
            .max_h(px(ROW_HEIGHT))
            .overflow_hidden()
            .bg(rgb(row_bg))
            .border_b_1()
            .border_color(rgb(DIVIDER))
            .child(topology_col),
    );

    if has_children && is_expanded {
        for child in &node.children {
            if *row_index >= MAX_DOPESHEET_ROWS {
                break;
            }
            hierarchy_stack = append_hierarchy_row_recursive(
                hierarchy_stack,
                view.clone(),
                workspace.clone(),
                ui_state,
                cx,
                child,
                depth + 1,
                row_index,
            );
        }
    }

    hierarchy_stack
}

fn stable_prim_element_key(path: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut hasher);
    hasher.finish()
}
