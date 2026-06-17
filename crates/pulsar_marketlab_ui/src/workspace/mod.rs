//! Nested resizable workstation layout: tri-pane row (stage composer, node canvas, inspector).

mod otl_editor_pane;
mod context;
mod graph_engine;
mod menu_bar;
mod node_canvas;
mod param_inspector;
mod split_layout;
mod dopesheet_state;
mod layer_control;
mod logical_topology;
mod stage_composer;
mod stage_ledger;
mod stage_tree_columns;
mod topology_dopesheet;
mod workstation_shelf;

pub use otl_editor_pane::{
    OtlEditorPane, WorkspaceTab, render_otl_editor, render_workspace_tab_bar,
};
pub use menu_bar::{MenuBar, MenuBarHost};

pub use node_canvas::{
    blender_slot_position, canvas_zoom_detail_level, compile_relationship_directive,
    execution_slot_for_target_prim, execution_slot_for_target_type,
    on_wire_disconnected, on_wire_released, paint_bezier_wire, paint_socket_dot,
    paint_wires_for_graph, render_canvas_single_line, render_collapsed_node_capsule,
    render_node_canvas, collapsed_pill_text_width, render_collapsed_pill_title,
    sanitize_node_label_text, truncate_node_header_title_at_runway, truncate_to_runway,
    CANVAS_ZOOM_COMPACT, CANVAS_ZOOM_MINIMAL, CanvasZoomDetailLevel,
    COLLAPSED_PILL_PAD_LEFT, COLLAPSED_PILL_PAD_RIGHT, truncate_node_header_title,
    NodeHeaderTitleBudget,
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
    GlobalPipelineOverview, ParamInspectorPane, render_composed_asset_metadata_grid,
    render_param_inspector,
};
pub use split_layout::{
    SplitHandle, SplitLayoutHost, WorkstationSplitLayout, render_split_handle,
};
pub use context::{
    ExecutionGraphCache, ManagedUsdPrim, ManagedUsdRelationship, ManagedUsdStage, ModelContext,
    PanelType, Point2D, WorkspaceContext, install_ui_selection_observer,
};
pub use graph_engine::{
    begin_graph_engine_timeline_sweep, build_composed_asset_registry, build_graph_compile_spec,
    build_path_binding_index, build_stage_graph_snapshot, build_stage_graph_snapshot_from_usda,
    composed_asset_meta_from_prim,
    install_graph_engine_invalidation_worker, spawn_graph_engine_timeline_sweep,
    GraphEngineInvalidationHost,
};
pub use dopesheet_state::DopesheetUiState;
pub use layer_control::{LayerDescriptor, LayerDisplayState, LayerStackControlState};
pub use logical_topology::{compile_logical_strategy_tree, LogicalTreeNode};
pub use stage_composer::{
    LayerStackPane, StageComposerPane, StagePrimRow, install_stage_composition_observer,
    install_stage_selection_observer, render_stage_composer_shelf_body,
    render_workstation_layer_stack,
};
pub use topology_dopesheet::{
    render_topology_dopesheet_panel, render_unified_bottom_topology_dopesheet,
    TopologyDopesheetHost,
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
pub use workstation_shelf::{
    render_collapsible_shelf, render_shelf_stack, WorkstationShelfHost, WorkstationShelfId,
    WorkstationShelfState,
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
    + WorkstationShelfHost
    + OtlEditorPane
    + MenuBarHost
    + NodeCanvasPane
    + GraphEngineInvalidationHost
    + LayerStackPane
    + 'static
{
    fn param_inspector_workspace(&self) -> Entity<WorkspaceContext>;
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
    let stage_composer_body = render_stage_composer_shelf_body(host, cx);
    let inspector_body = render_param_inspector(view.clone(), host, window, cx);
    let otl_body = render_otl_editor(view.clone(), host, window, cx);
    let layer_stack = render_workstation_layer_stack(
        view.clone(),
        host.param_inspector_workspace().clone(),
        cx,
    );
    let shelves = host.workstation_shelf_state().clone();
    let context_tower = render_shelf_stack(
        div()
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .child(render_collapsible_shelf(
                view.clone(),
                &shelves,
                WorkstationShelfId::TowerInspector,
                "Param Inspector",
                true,
                inspector_body,
            ))
            .child(render_collapsible_shelf(
                view.clone(),
                &shelves,
                WorkstationShelfId::TowerOtlEditor,
                "OTL Script Editor",
                true,
                otl_body,
            ))
            .child(render_collapsible_shelf(
                view.clone(),
                &shelves,
                WorkstationShelfId::TowerLayerStack,
                "USD Layer Stack",
                false,
                layer_stack,
            ))
            .child(render_collapsible_shelf(
                view.clone(),
                &shelves,
                WorkstationShelfId::TowerStageComposer,
                "Stage Composer",
                true,
                stage_composer_body,
            )),
    );
    let center_body: AnyElement = div()
        .id("node-canvas-viewport")
        .relative()
        .size_full()
        .min_h_0()
        .min_w_0()
        .child(node_canvas::render_node_canvas(view.clone(), host, cx))
        .into_any_element();
    let bottom_dopesheet =
        render_unified_bottom_topology_dopesheet(view.clone(), host, cx);

    div()
        .id("marketlab-workstation-root")
        .relative()
        .size_full()
        .flex()
        .flex_col()
        .bg(rgb(theme::WORKSTATION_ROOT))
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
                        if let Some(container_bounds) = union_bounds(&bounds) {
                            view.update(cx, |host, _cx| {
                                host.set_split_container_bounds(container_bounds);
                            });
                        }
                    }
                })
                .id("marketlab-workstation-split-container")
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .child(
                    div()
                        .on_children_prepainted({
                            let view = view.clone();
                            move |bounds: Vec<Bounds<Pixels>>, _window: &mut Window, cx: &mut App| {
                                if let Some(upper_bounds) = union_bounds(&bounds) {
                                    view.update(cx, |host, _cx| {
                                        host.set_upper_row_bounds(upper_bounds);
                                    });
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
                        .flex_1()
                        .min_w(px(180.0))
                        .min_h_0()
                        .child(center_body),
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
                        .child(
                            div()
                                .size_full()
                                .min_h_0()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .child(
                                    div()
                                        .flex_shrink_0()
                                        .px_3()
                                        .py_2()
                                        .border_b_1()
                                        .border_color(crate::theme::chrome_color(
                                            crate::theme::GRID_MAJOR,
                                        ))
                                        .text_xs()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .font_family("monospace")
                                        .text_color(crate::theme::chrome_color(
                                            crate::theme::TEXT_SECONDARY,
                                        ))
                                        .child("Context Tower"),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_h_0()
                                        .min_w_0()
                                        .flex()
                                        .flex_col()
                                        .child(context_tower),
                                ),
                        ),
                ),
                )
                .child(render_split_handle(
                    view.clone(),
                    SplitHandle::MainBottom,
                    Axis::Vertical,
                ))
                .child(
                    div()
                        .id("marketlab-workstation-bottom")
                        .flex_shrink_0()
                        .h(relative(layout.bottom_share))
                        .min_h(px(96.0))
                        .max_h(relative(0.72))
                        .min_w_0()
                        .child(bottom_dopesheet),
                ),
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
