//! Nested resizable workstation layout: tri-pane row (stage composer, node canvas, inspector).

mod otl_editor_pane;
mod context;
mod graph_engine;
mod menu_bar;
mod node_canvas;
mod param_inspector;
mod render_viewport;
mod split_layout;
mod stage_composer;
mod stage_ledger;
mod stage_tree_columns;

pub use otl_editor_pane::{
    OtlEditorPane, WorkspaceTab, render_otl_editor, render_workspace_tab_bar,
};
pub use menu_bar::{MenuBar, MenuBarHost};

pub use node_canvas::{
    blender_slot_position, compile_relationship_directive, execution_slot_for_target_prim,
    execution_slot_for_target_type,
    on_wire_disconnected, on_wire_released, paint_bezier_wire, paint_socket_dot,
    paint_wires_for_graph, render_collapsed_node_capsule, render_node_canvas,
    render_wiring_alerts, socket_color, socket_pin, paint_dcc_canvas_grid, CanvasEnvironmentTab, CapsuleSocketSide,
    ExecutionSlotKind, GraphWireSegment, NodeCanvasPane, SocketWireKind, StageRelationshipDirective,
    BLENDER_COLUMN_WIDTH, BLENDER_ORIGIN_X, BLENDER_ORIGIN_Y, BLENDER_ROW_HEIGHT, DCC_BORDER,
    DCC_CANVAS_BACKPLATE, DCC_CAPSULE_HEIGHT, DCC_CAPSULE_WIDTH, DCC_HEADER_ACTIVE,
    DCC_NODE_CORNER_RADIUS_PX, DCC_NODE_HULL, DCC_NODE_SELECTED, DCC_TEXT_PRIMARY,
    DCC_TEXT_SECONDARY, capsule_socket_world_center,
};
pub use crate::theme::{NODE_SELECTION_HALO, GRID_MAJOR_SPACING_PX, GRID_MINOR_SPACING_PX};
pub use crate::theme;
pub use param_inspector::{
    GlobalPipelineOverview, ParamInspectorPane, render_param_inspector,
};
pub use render_viewport::{LedgerRow, RenderViewportPane, render_render_viewport};
// Render viewport retained for future GUI; not mounted in the current layout.
pub use split_layout::{
    SplitHandle, SplitLayoutHost, WorkstationSplitLayout, render_split_handle,
};
pub use context::{
    ExecutionGraphCache, ManagedUsdPrim, ManagedUsdRelationship, ManagedUsdStage, ModelContext,
    PanelType, Point2D, WorkspaceContext, install_ui_selection_observer,
};
pub use graph_engine::{
    begin_graph_engine_timeline_sweep, build_graph_compile_spec, build_stage_graph_snapshot,
    install_graph_engine_invalidation_worker, spawn_graph_engine_timeline_sweep,
    GraphEngineInvalidationHost,
};
pub use stage_composer::{
    StageComposerPane, StagePrimRow, install_stage_composition_observer,
    install_stage_selection_observer, render_stage_composer,
};
pub use stage_tree_columns::{
    StageTreeColumnHandle, StageTreeColumnHost, StageTreeColumnLayout,
    STAGE_TREE_ACTIVE_WIDTH, STAGE_TREE_CHEVRON_WIDTH, STAGE_TREE_COLUMN_MIN,
    STAGE_TREE_PATH_MIN, render_column_splitter,
};
pub use stage_ledger::{
    StageLedgerEntry, StageLedgerExplorer, install_workspace_context_observer,
    render_stage_ledger,
};

use gpui::*;

fn union_bounds(bounds: &[Bounds<Pixels>]) -> Option<Bounds<Pixels>> {
    let mut iter = bounds.iter();
    let first = *iter.next()?;
    Some(iter.fold(first, |acc, next| {
        let top_left = point(
            acc.origin.x.min(next.origin.x),
            acc.origin.y.min(next.origin.y),
        );
        let bottom_right = point(
            acc.bottom_right().x.max(next.bottom_right().x),
            acc.bottom_right().y.max(next.bottom_right().y),
        );
        Bounds::from_corners(top_left, bottom_right)
    }))
}

/// Host type rendered inside the workstation panes.
pub trait WorkstationLayoutHost:
    Sized
    + SplitLayoutHost
    + StageComposerPane
    + StageTreeColumnHost
    + ParamInspectorPane
    + OtlEditorPane
    + MenuBarHost
    + NodeCanvasPane
    + GraphEngineInvalidationHost
    + 'static
{
}

/// Build the nested splitter tree with GPUI drag-captured handles.
pub fn render_workstation_layout<H: WorkstationLayoutHost>(
    host: &mut H,
    window: &mut Window,
    cx: &mut Context<H>,
) -> impl IntoElement {
    let view = cx.entity();
    let layout = host.split_layout().clamp();
    let menu_bar = MenuBar::render(view.clone(), host, window, cx);
    let stage_composer = render_stage_composer(view.clone(), host, cx);
    let active_tab = host.active_workspace_tab();
    let right_pane_body: AnyElement = match active_tab {
        WorkspaceTab::ParamInspector => {
            render_param_inspector(view.clone(), host, window, cx).into_any_element()
        }
        WorkspaceTab::OtlEditor => {
            render_otl_editor(view.clone(), host, window, cx).into_any_element()
        }
    };
    let right_pane_tabs = render_workspace_tab_bar(view.clone(), host);
    let node_canvas = node_canvas::render_node_canvas(view.clone(), host, cx);

    div()
        .on_children_prepainted({
            let view = view.clone();
            move |bounds: Vec<Bounds<Pixels>>, _window: &mut Window, cx: &mut App| {
                if let Some(root_bounds) = union_bounds(&bounds) {
                    view.update(cx, |host, _cx| host.set_split_container_bounds(root_bounds));
                }
            }
        })
        .id("marketlab-workstation-root")
        .relative()
        .size_full()
        .flex()
        .flex_col()
        .bg(rgb(0x09090b))
        .on_key_down({
            let view = view.clone();
            move |event: &KeyDownEvent, _window: &mut Window, cx: &mut App| {
                if event.keystroke.modifiers.control && event.keystroke.key.as_str() == "b" {
                    view.update(cx, |host, cx| host.compile_otl_script(cx));
                    return;
                }
                if !event.keystroke.modifiers.control {
                    return;
                }
                view.update(cx, |host, cx| match event.keystroke.key.as_str() {
                    "n" => host.on_file_new(cx),
                    "o" => host.on_file_open(cx),
                    "s" if event.keystroke.modifiers.shift => host.on_file_save_as(cx),
                    "s" => host.on_file_save(cx),
                    _ => {}
                });
            }
        })
        .child(menu_bar)
        .child(
            div()
                .on_children_prepainted({
                    let view = view.clone();
                    move |bounds: Vec<Bounds<Pixels>>, _window: &mut Window, cx: &mut App| {
                        if let Some(upper_bounds) = union_bounds(&bounds) {
                            view.update(cx, |host, _cx| host.set_upper_row_bounds(upper_bounds));
                        }
                    }
                })
                .id("marketlab-workstation-main")
                .relative()
                .flex_1()
                .min_h_0()
                .flex()
                .flex_row()
                .child(
                    div()
                        .flex_shrink_0()
                        .w(relative(layout.stage_share))
                        .min_w(px(140.0))
                        .child(pane_shell("Stage Composer", stage_composer)),
                )
                .child(render_split_handle(
                    view.clone(),
                    SplitHandle::StageCanvas,
                    Axis::Horizontal,
                ))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(180.0))
                        .child(pane_shell("Node Canvas", node_canvas)),
                )
                .child(render_split_handle(
                    view.clone(),
                    SplitHandle::CanvasInspector,
                    Axis::Horizontal,
                ))
                .child(
                    div()
                        .flex_shrink_0()
                        .w(relative(layout.inspector_share))
                        .min_w(px(220.0))
                        .child(pane_shell_with_tabs(
                            active_tab.label(),
                            right_pane_tabs,
                            right_pane_body,
                        )),
                ),
        )
}

fn pane_shell_with_tabs(
    title: &'static str,
    tabs: impl IntoElement,
    body: impl IntoElement,
) -> impl IntoElement {
    div()
        .size_full()
        .min_h_0()
        .min_w_0()
        .flex()
        .flex_col()
        .bg(crate::theme::chrome_color(crate::theme::PANE_BACKPLATE))
        .border_1()
        .border_color(crate::theme::chrome_color(crate::theme::GRID_MAJOR))
        .child(
            div()
                .flex_shrink_0()
                .px_3()
                .py_2()
                .border_b_1()
                .border_color(crate::theme::chrome_color(crate::theme::GRID_MAJOR))
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .font_family("monospace")
                .text_color(crate::theme::chrome_color(crate::theme::TEXT_SECONDARY))
                .child(title),
        )
        .child(tabs)
        .child(
            div()
                .flex_1()
                .min_h_0()
                .min_w_0()
                .flex()
                .flex_col()
                .child(body),
        )
}

fn pane_shell(title: &'static str, body: impl IntoElement) -> impl IntoElement {
    div()
        .size_full()
        .min_h_0()
        .min_w_0()
        .flex()
        .flex_col()
        .bg(crate::theme::chrome_color(crate::theme::PANE_BACKPLATE))
        .border_1()
        .border_color(crate::theme::chrome_color(crate::theme::GRID_MAJOR))
        .child(
            div()
                .flex_shrink_0()
                .px_3()
                .py_2()
                .border_b_1()
                .border_color(crate::theme::chrome_color(crate::theme::GRID_MAJOR))
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .font_family("monospace")
                .text_color(crate::theme::chrome_color(crate::theme::TEXT_SECONDARY))
                .child(title),
        )
        .child(
            div()
                .flex_1()
                .min_h_0()
                .min_w_0()
                .flex()
                .flex_col()
                .child(body),
        )
}
