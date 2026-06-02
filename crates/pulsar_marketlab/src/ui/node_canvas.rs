//! Pipeline node canvas: dragging, wiring, context menus, and wire painting.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use gpui::*;
use gpui::prelude::FluentBuilder;

use gpui_component::menu::{ContextMenuExt, DropdownMenu, PopupMenuItem};

use openusd::sdf::Value;

use crate::graph_compiler::{
    apply_canonical_ta_ports, effective_otl_script, input_port_kind, output_port_kind,
    portfolio_signal_port_label, sync_otl_shader_ports_from_script, NodeConnection,
    portfolio_wired_source_count, connection_is_valid, input_port_is_wired, input_port_world_center,
    node_shows_price_chart, output_port_is_wired, output_port_world_center,
    portfolio_ensure_spare_input_port, portfolio_resolve_input_port, NodeGradeType, NodeType,
    OtlStdlibPreset, OTL_STDLIB_PRESETS, PortWireKind, VisualNode, CONNECTION_STROKE_WIDTH,
    MAX_ZOOM, MIN_ZOOM, NODE_CHART_HEIGHT, NODE_WIDTH, WIRE_PORT_HIT_RADIUS, ZOOM_WHEEL_SENSITIVITY,
};
use crate::ui::ta_uber_inspector::{archetype_summary, ta_header_tint};
use pulsar_marketlab_core::{node_display_name, parse_script_scalar_uniforms, TaArchetype, TaUberSignalConfig};
use pulsar_marketlab_ui::workspace::{
    blender_slot_position, paint_dcc_canvas_grid, paint_socket_dot, socket_pin, NodeCanvasPane,
    SocketWireKind, DCC_BORDER, DCC_CANVAS_BACKPLATE, DCC_CAPSULE_HEIGHT,
    DCC_CAPSULE_WIDTH, DCC_HEADER_ACTIVE, DCC_NODE_CORNER_RADIUS_PX, DCC_NODE_HULL,
    DCC_NODE_SELECTED, DCC_TEXT_PRIMARY, NODE_SELECTION_HALO,
};
use pulsar_marketlab_ui::{node_dropdown_trigger, NodeNumberInput};
use crate::workspace_state::{
    parse_chart_date_ordinal, ChartHistoryBuffer, CHART_Y_MIN_SPAN, CHART_Y_PADDING_RATIO,
    format_currency, format_percent_signed, format_ratio,
    TaExecutionBridge, TradingSystemWorkspace, CHART_STROKE_WIDTH,
};

const AGGREGATOR_DOUBLE_CLICK_WINDOW: Duration = Duration::from_millis(400);

const PORTFOLIO_ALLOCATION_OPTIONS: &[(&str, &str)] = &[
    ("Allocation::HierarchicalRiskParity", "Hierarchical Risk Parity"),
    ("Allocation::EqualWeight", "Equal Weight"),
    ("Allocation::MeanVariance", "Mean Variance"),
];

fn upstream_subgraph_node_ids(
    aggregator_id: usize,
    connections: &[NodeConnection],
) -> HashSet<usize> {
    let mut ids = HashSet::new();
    ids.insert(aggregator_id);
    let mut frontier = vec![aggregator_id];
    while let Some(to_id) = frontier.pop() {
        for connection in connections {
            if connection.to_node_id == to_id && ids.insert(connection.from_node_id) {
                frontier.push(connection.from_node_id);
            }
        }
    }
    ids
}

fn socket_kind_from_port(kind: PortWireKind) -> SocketWireKind {
    match kind {
        PortWireKind::StructuralPath => SocketWireKind::StructuralPath,
        PortWireKind::NumericSignal => SocketWireKind::NumericSignal,
        PortWireKind::Aov => SocketWireKind::Aov,
    }
}

fn union_canvas_bounds(bounds: &[Bounds<Pixels>]) -> Option<Bounds<Pixels>> {
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

impl TradingSystemWorkspace {
    fn canvas_scope_node_id(&self) -> Option<usize> {
        self.canvas_tabs
            .get(self.active_canvas_tab)
            .and_then(|tab| tab.scope_node_id)
    }

    fn canvas_visible_node_ids(&self) -> HashSet<usize> {
        match self.canvas_scope_node_id() {
            None => self.nodes.iter().map(|node| node.id).collect(),
            Some(aggregator_id) => {
                upstream_subgraph_node_ids(aggregator_id, &self.connections)
            }
        }
    }

    fn canvas_visible_nodes(&self) -> Vec<VisualNode> {
        let visible = self.canvas_visible_node_ids();
        self.nodes
            .iter()
            .filter(|node| visible.contains(&node.id))
            .cloned()
            .collect()
    }

    fn canvas_visible_connections(&self) -> Vec<NodeConnection> {
        let visible = self.canvas_visible_node_ids();
        self.connections
            .iter()
            .filter(|connection| {
                visible.contains(&connection.from_node_id)
                    && visible.contains(&connection.to_node_id)
            })
            .cloned()
            .collect()
    }

    fn handle_aggregator_header_click(&mut self, node_id: usize, cx: &mut Context<Self>) {
        let Some(node) = self.nodes.iter().find(|node| node.id == node_id) else {
            return;
        };
        if !node.node_type.is_portfolio() {
            return;
        }
        let now = Instant::now();
        if let Some((last_id, last_time)) = self.last_node_header_click {
            if last_id == node_id && now.duration_since(last_time) <= AGGREGATOR_DOUBLE_CLICK_WINDOW
            {
                let label = node.name.clone();
                let scope_path = self
                    .resolved_stage_path_for_node(node)
                    .unwrap_or_else(|| node.name.clone());
                self.open_aggregator_canvas(node_id, label, scope_path, cx);
                self.last_node_header_click = None;
                return;
            }
        }
        self.last_node_header_click = Some((node_id, now));
    }

    pub(crate) fn canvas_local_position(&self, position: Point<Pixels>) -> (f32, f32) {
        let mouse_x: f32 = position.x.into();
        let mouse_y: f32 = position.y.into();
        let canvas_x: f32 = self.canvas_origin.x.into();
        let canvas_y: f32 = self.canvas_origin.y.into();
        (mouse_x - canvas_x, mouse_y - canvas_y)
    }

    pub(crate) fn screen_to_world(&self, local_x: f32, local_y: f32) -> (f32, f32) {
        let pan_x: f32 = self.pan_offset.x.into();
        let pan_y: f32 = self.pan_offset.y.into();
        (
            (local_x - pan_x) / self.zoom_scale,
            (local_y - pan_y) / self.zoom_scale,
        )
    }

    fn world_to_screen(&self, world_x: f32, world_y: f32) -> (f32, f32) {
        let pan_x: f32 = self.pan_offset.x.into();
        let pan_y: f32 = self.pan_offset.y.into();
        (
            world_x * self.zoom_scale + pan_x,
            world_y * self.zoom_scale + pan_y,
        )
    }

    fn apply_scroll_zoom(&mut self, event: &ScrollWheelEvent) {
        let delta_y: f32 = event.delta.pixel_delta(px(16.0)).y.into();
        if delta_y.abs() < f32::EPSILON {
            return;
        }

        let (local_x, local_y) = self.canvas_local_position(event.position);
        let pan_x: f32 = self.pan_offset.x.into();
        let pan_y: f32 = self.pan_offset.y.into();
        let old_scale = self.zoom_scale;
        let new_scale = (old_scale * (1.0 - delta_y * ZOOM_WHEEL_SENSITIVITY))
            .clamp(MIN_ZOOM, MAX_ZOOM);

        if (new_scale - old_scale).abs() < f32::EPSILON {
            return;
        }

        let world_x = (local_x - pan_x) / old_scale;
        let world_y = (local_y - pan_y) / old_scale;
        self.zoom_scale = new_scale;
        self.pan_offset = point(
            px(local_x - world_x * new_scale),
            px(local_y - world_y * new_scale),
        );
    }

    fn begin_pan(&mut self, position: Point<Pixels>) {
        self.is_panning = true;
        let (local_x, local_y) = self.canvas_local_position(position);
        self.last_pan_mouse_pos = point(px(local_x), px(local_y));
    }

    fn update_pan(&mut self, position: Point<Pixels>) {
        let (local_x, local_y) = self.canvas_local_position(position);
        let last_x: f32 = self.last_pan_mouse_pos.x.into();
        let last_y: f32 = self.last_pan_mouse_pos.y.into();
        let pan_x: f32 = self.pan_offset.x.into();
        let pan_y: f32 = self.pan_offset.y.into();
        self.pan_offset = point(
            px(pan_x + local_x - last_x),
            px(pan_y + local_y - last_y),
        );
        self.last_pan_mouse_pos = point(px(local_x), px(local_y));
    }

    fn end_pan(&mut self) {
        self.is_panning = false;
    }

    fn begin_node_drag(&mut self, node_id: usize, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some((node_x, node_y)) = self
            .nodes
            .iter()
            .find(|node| node.id == node_id)
            .map(|node| (node.x, node.y))
        else {
            return;
        };

        let (local_x, local_y) = self.canvas_local_position(position);
        let (world_x, world_y) = self.screen_to_world(local_x, local_y);
        if self.selected_node_id != Some(node_id) {
            if let Some(prim_path) = self
                .nodes
                .iter()
                .find(|node| node.id == node_id)
                .and_then(|node| self.resolved_stage_path_for_node(node))
            {
                self.select_stage_path(Some(prim_path), cx);
            } else {
                self.selected_node_id = Some(node_id);
                self.sync_inspector_from_selection(cx);
            }
        }
        self.active_drag_node_id = Some(node_id);
        self.drag_offset = point(
            px(world_x - node_x),
            px(world_y - node_y),
        );
    }

    fn update_dragged_node_position(&mut self, position: Point<Pixels>) {
        let Some(node_id) = self.active_drag_node_id else {
            return;
        };

        let (local_x, local_y) = self.canvas_local_position(position);
        let (world_x, world_y) = self.screen_to_world(local_x, local_y);
        let offset_x: f32 = self.drag_offset.x.into();
        let offset_y: f32 = self.drag_offset.y.into();
        let new_x = world_x - offset_x;
        let new_y = world_y - offset_y;

        if let Some(node) = self.nodes.iter_mut().find(|node| node.id == node_id) {
            node.x = new_x;
            node.y = new_y;
        }
    }

    fn end_node_drag(&mut self) {
        self.active_drag_node_id = None;
    }

    fn open_context_menu(&mut self, position: Point<Pixels>) {
        let (local_x, local_y) = self.canvas_local_position(position);
        let (world_x, world_y) = self.screen_to_world(local_x, local_y);
        self.context_menu_pos = Some(point(px(world_x), px(world_y)));
    }

    fn dismiss_context_menu(&mut self) {
        self.context_menu_pos = None;
    }
    pub(crate) fn next_node_id(&self) -> usize {
        self.nodes.iter().map(|node| node.id).max().unwrap_or(0) + 1
    }

    fn spawn_ta_uber_archetype(&mut self, archetype: TaArchetype, cx: &mut Context<Self>) {
        let Some(menu_pos) = self.context_menu_pos else {
            return;
        };

        let node_id = self.next_node_id();
        let ta_index = self
            .nodes
            .iter()
            .filter(|node| node.node_type.is_ta_uber_signal())
            .count();
        let (column_x, _) = blender_slot_position(1, ta_index);
        let x = column_x;
        let y: f32 = menu_pos.y.into();

        let config = TaUberSignalConfig::new(archetype);
        let name = node_display_name(&config);

        let mut node = VisualNode {
            id: node_id,
            name,
            node_type: NodeType::ta_uber_signal(config),
            grade: NodeGradeType::Scalar,
            portfolio_allocation_id: None,
            dsl_formula: None,
            aov_outputs: Vec::new(),
            asset_source: None,
            x,
            y,
            collapsed: false,
            inputs: Vec::new(),
            outputs: Vec::new(),
        };
        apply_canonical_ta_ports(&mut node);
        self.nodes.push(node);
        self.selected_node_id = Some(node_id);
        self.context_menu_pos = None;
        self.sync_pipeline_graph(cx);
        self.invalidate_playhead_evaluation_cache();
        cx.notify();
    }

    fn spawn_otl_stdlib_preset(&mut self, preset: &OtlStdlibPreset, cx: &mut Context<Self>) {
        let Some(menu_pos) = self.context_menu_pos else {
            return;
        };

        let node_id = self.next_node_id();
        let shader_index = self
            .nodes
            .iter()
            .filter(|node| node.node_type.is_otl_shader())
            .count();
        let (column_x, _) = blender_slot_position(1, shader_index);
        let x = column_x;
        let y: f32 = menu_pos.y.into();

        let mut node = VisualNode {
            id: node_id,
            name: preset.display_name.to_string(),
            node_type: NodeType::otl_shader(preset.default_script),
            grade: NodeGradeType::Scalar,
            portfolio_allocation_id: None,
            dsl_formula: None,
            aov_outputs: Vec::new(),
            asset_source: None,
            x,
            y,
            collapsed: false,
            inputs: Vec::new(),
            outputs: Vec::new(),
        };
        let _ = sync_otl_shader_ports_from_script(
            &mut node,
            preset.default_script,
            &mut self.connections,
        );
        self.nodes.push(node);
        self.selected_node_id = Some(node_id);
        self.context_menu_pos = None;
        self.sync_pipeline_graph(cx);
        self.invalidate_playhead_evaluation_cache();
        cx.notify();
    }

    fn spawn_csv_asset_node(&mut self, cx: &mut Context<Self>) {
        let Some(menu_pos) = self.context_menu_pos else {
            return;
        };

        let node_id = self.next_node_id();
        let asset_index = self
            .nodes
            .iter()
            .filter(|node| node.node_type.is_asset_adaptor())
            .count();
        let (column_x, _) = blender_slot_position(0, asset_index);
        let x = column_x;
        let y: f32 = menu_pos.y.into();

        self.nodes.push(VisualNode {
            id: node_id,
            name: "CSV Asset".to_string(),
            node_type: NodeType::asset_adaptor_from_label("CSV Asset"),
            grade: NodeGradeType::Scalar,
            portfolio_allocation_id: None,
            dsl_formula: None,
            aov_outputs: Vec::new(),
            asset_source: None,
            x,
            y,
            collapsed: false,
            inputs: vec![],
            outputs: vec!["Close Out".to_string()],
        });
        self.selected_node_id = Some(node_id);
        self.asset_path_input.update(cx, |input, cx| {
            input.set_content(String::new(), cx);
        });
        self.context_menu_pos = None;
        self.sync_pipeline_graph(cx);
        self.invalidate_playhead_evaluation_cache();
        self.prompt_csv_for_node(node_id, cx);
    }

    fn spawn_portfolio_node(&mut self, cx: &mut Context<Self>) {
        let Some(menu_pos) = self.context_menu_pos else {
            return;
        };

        let node_id = self.next_node_id();
        let portfolio_index = self
            .nodes
            .iter()
            .filter(|node| node.node_type.is_portfolio())
            .count();
        let (column_x, _) = blender_slot_position(2, portfolio_index);
        let x = column_x;
        let y: f32 = menu_pos.y.into();

        self.nodes.push(VisualNode {
            id: node_id,
            name: format!("Sim Portfolio {}", portfolio_index + 1),
            node_type: NodeType::portfolio(),
            grade: NodeGradeType::Scalar,
            portfolio_allocation_id: Some("Allocation::HierarchicalRiskParity".to_string()),
            dsl_formula: None,
            aov_outputs: Vec::new(),
            asset_source: None,
            x,
            y,
            collapsed: false,
            inputs: vec![portfolio_signal_port_label(0)],
            outputs: vec!["NAV Out".to_string()],
        });
        self.selected_node_id = Some(node_id);
        self.context_menu_pos = None;
        self.sync_pipeline_graph(cx);
        self.invalidate_playhead_evaluation_cache();
        cx.notify();
    }

    pub(crate) fn portfolio_wired_sources(&self, portfolio_id: usize) -> Vec<(usize, String)> {
        self.connections
            .iter()
            .filter(|connection| connection.to_node_id == portfolio_id)
            .filter_map(|connection| {
                self.nodes
                    .iter()
                    .find(|node| node.id == connection.from_node_id)
                    .map(|node| (node.id, node.name.clone()))
            })
            .collect()
    }

    fn begin_wire_from_output(
        &mut self,
        node_id: usize,
        port_idx: usize,
        position: Point<Pixels>,
    ) {
        self.active_wire_source = Some((node_id, port_idx));
        self.active_mouse_pos = position;
    }

    fn update_wire_tracking(&mut self, position: Point<Pixels>) {
        self.active_mouse_pos = position;
    }

    fn cancel_wire(&mut self) {
        self.active_wire_source = None;
    }

    fn disconnect_input_wire(&mut self, to_node_id: usize, to_port_idx: usize, cx: &mut Context<Self>) {
        let Some(connection) = self
            .connections
            .iter()
            .find(|connection| {
                connection.to_node_id == to_node_id && connection.to_port_idx == to_port_idx
            })
            .cloned()
        else {
            return;
        };

        let from_node_id = connection.from_node_id;
        let source_prim = self
            .nodes
            .iter()
            .find(|node| node.id == from_node_id)
            .and_then(|node| self.resolved_stage_path_for_node(node));
        let target_prim = self
            .nodes
            .iter()
            .find(|node| node.id == to_node_id)
            .and_then(|node| self.resolved_stage_path_for_node(node));
        let from_is_ta = self
            .nodes
            .iter()
            .find(|node| node.id == from_node_id)
            .is_some_and(|node| node.node_type.is_ta_uber_signal());

        self.connections.retain(|connection| {
            !(connection.to_node_id == to_node_id && connection.to_port_idx == to_port_idx)
        });

        if let (Some(source), Some(target)) = (source_prim.as_deref(), target_prim.as_deref()) {
            self.disconnect_primitives(source, target, cx);
        }

        if from_is_ta {
            let mut bridge = TaExecutionBridge::new();
            bridge.clear_ta_signal_slot(from_node_id, &mut self.market_stage);
        }

        self.push_status_log(format!(
            "Wire disconnected — node {from_node_id} → node {to_node_id} port {to_port_idx}"
        ));
        self.sync_pipeline_graph(cx);
        self.invalidate_playhead_evaluation_cache();
        self.recompute_playhead_diagnostics();
        cx.notify();
    }

    fn commit_wire_to_input(
        &mut self,
        to_node_id: usize,
        to_port_idx: usize,
        cx: &mut Context<Self>,
    ) -> Option<(usize, usize)> {
        let Some((from_node_id, from_port_idx)) = self.active_wire_source else {
            return None;
        };

        if from_node_id == to_node_id {
            self.active_wire_source = None;
            return None;
        }

        let Some(from_node) = self.nodes.iter().find(|node| node.id == from_node_id).cloned() else {
            self.active_wire_source = None;
            return None;
        };
        let Some(to_node) = self.nodes.iter().find(|node| node.id == to_node_id).cloned() else {
            self.active_wire_source = None;
            return None;
        };

        if !connection_is_valid(&from_node, from_port_idx, &to_node, to_port_idx) {
            self.push_status_log(format!(
                "Wire rejected — {:?} cannot connect to {:?}",
                from_node.node_type, to_node.node_type
            ));
            self.active_wire_source = None;
            return None;
        }

        let effective_port = if to_node.node_type.is_portfolio() {
            if self.connections.iter().any(|connection| {
                connection.to_node_id == to_node_id && connection.from_node_id == from_node_id
            }) {
                self.push_status_log(format!(
                    "Wire rejected — node {from_node_id} already feeds portfolio {to_node_id}"
                ));
                self.active_wire_source = None;
                return None;
            }
            let Some(port_idx) = portfolio_resolve_input_port(
                &mut self.nodes,
                &self.connections,
                to_node_id,
                to_port_idx,
            ) else {
                self.active_wire_source = None;
                return None;
            };
            port_idx
        } else {
            self.connections.retain(|connection| {
                connection.to_node_id != to_node_id || connection.to_port_idx != to_port_idx
            });
            to_port_idx
        };

        self.connections.push(NodeConnection {
            from_node_id,
            from_port_idx,
            to_node_id,
            to_port_idx: effective_port,
        });
        if to_node.node_type.is_portfolio() {
            portfolio_ensure_spare_input_port(&mut self.nodes, &self.connections, to_node_id);
        }
        self.active_wire_source = None;
        self.sync_pipeline_graph(cx);
        Some((effective_port, from_node_id))
    }

    fn try_commit_wire_to_input(&mut self, to_node_id: usize, to_port_idx: usize, cx: &mut Context<Self>) {
        if let Some((port, from_node_id)) = self.commit_wire_to_input(to_node_id, to_port_idx, cx) {
            let source_prim = self
                .nodes
                .iter()
                .find(|node| node.id == from_node_id)
                .and_then(|node| self.resolved_stage_path_for_node(node));
            let target_prim = self
                .nodes
                .iter()
                .find(|node| node.id == to_node_id)
                .and_then(|node| self.resolved_stage_path_for_node(node));
            if let (Some(source), Some(target)) = (source_prim.as_deref(), target_prim.as_deref()) {
                self.connect_primitives(source, target, cx);
            }
            self.push_status_log(format!(
                "Wire connected → node {to_node_id} port {port} (from node {from_node_id})"
            ));
            self.invalidate_playhead_evaluation_cache();
            self.recompute_playhead_diagnostics();
            cx.notify();
        }
    }

    fn node_includes_chart(&self, node: &VisualNode) -> bool {
        node_shows_price_chart(node)
            && self
                .asset_chart_history
                .get(&node.id)
                .map(|buffer| buffer.values.len() >= 2)
                .unwrap_or(false)
    }

    fn find_wire_drop_target(&self, screen_pos: Point<Pixels>) -> Option<(usize, usize)> {
        let (local_x, local_y) = self.canvas_local_position(screen_pos);
        let hit_radius_sq = WIRE_PORT_HIT_RADIUS * WIRE_PORT_HIT_RADIUS;

        for node in self.canvas_visible_nodes().iter() {
            let include_chart = self.node_includes_chart(node);
            for (port_idx, _) in node.inputs.iter().enumerate() {
                if node.collapsed
                    && !input_port_is_wired(node, port_idx, &self.connections)
                {
                    continue;
                }
                let (port_world_x, port_world_y) = input_port_world_center(
                    node,
                    port_idx,
                    include_chart,
                    &self.connections,
                );
                let (port_screen_x, port_screen_y) = self.world_to_screen(port_world_x, port_world_y);
                let dx = local_x - port_screen_x;
                let dy = local_y - port_screen_y;
                if dx * dx + dy * dy <= hit_radius_sq {
                    return Some((node.id, port_idx));
                }
            }
        }
        None
    }

    fn is_near_active_wire_source(&self, screen_pos: Point<Pixels>) -> bool {
        let Some((from_node_id, from_port_idx)) = self.active_wire_source else {
            return false;
        };
        let Some(node) = self.nodes.iter().find(|node| node.id == from_node_id) else {
            return false;
        };
        let (local_x, local_y) = self.canvas_local_position(screen_pos);
        let (port_world_x, port_world_y) = output_port_world_center(
            node,
            from_port_idx,
            self.node_includes_chart(node),
            &self.connections,
        );
        let (port_screen_x, port_screen_y) = self.world_to_screen(port_world_x, port_world_y);
        let dx = local_x - port_screen_x;
        let dy = local_y - port_screen_y;
        dx * dx + dy * dy <= WIRE_PORT_HIT_RADIUS * WIRE_PORT_HIT_RADIUS
    }

    fn handle_canvas_left_mouse_up(&mut self, screen_pos: Point<Pixels>, cx: &mut Context<Self>) {
        if self.ta_lookback_scrubbing {
            self.end_ta_lookback_scrub(cx);
        }

        if self.active_drag_node_id.is_some() {
            self.end_node_drag();
            cx.notify();
            return;
        }

        if self.active_wire_source.is_some() {
            if let Some((to_node_id, to_port_idx)) = self.find_wire_drop_target(screen_pos) {
                self.try_commit_wire_to_input(to_node_id, to_port_idx, cx);
            } else if self.is_near_active_wire_source(screen_pos) {
                // Click-to-connect: keep wire armed after releasing on the source port.
                return;
            } else {
                self.cancel_wire();
                cx.notify();
            }
        }
    }

    fn paint_bezier_wire(
        bounds: Bounds<Pixels>,
        start_x: f32,
        start_y: f32,
        end: Point<Pixels>,
        window: &mut Window,
        stroke: impl Into<Background>,
    ) {
        let stroke = stroke.into();
        let start = Self::canvas_point(bounds, start_x, start_y);
        let start_x_px: f32 = start.x.into();
        let end_x_px: f32 = end.x.into();
        let mid_x = (start_x_px + end_x_px) / 2.0;

        let mut builder = PathBuilder::stroke(px(CONNECTION_STROKE_WIDTH));
        builder.move_to(start);
        builder.cubic_bezier_to(
            end,
            point(px(mid_x), start.y),
            point(px(mid_x), end.y),
        );

        if let Ok(path) = builder.build() {
            window.paint_path(path, stroke);
        }
    }

    fn paint_asset_price_chart(
        bounds: Bounds<Pixels>,
        buffer: &ChartHistoryBuffer,
        window: &mut Window,
        stroke: impl Into<Background>,
    ) {
        if buffer.values.len() < 2 || buffer.timestamps.len() != buffer.values.len() {
            return;
        }

        let stroke = stroke.into();
        let x_coords: Vec<f32> = buffer
            .timestamps
            .iter()
            .filter_map(|date| parse_chart_date_ordinal(date))
            .collect();
        if x_coords.len() != buffer.values.len() {
            return;
        }

        let min_value = buffer
            .values
            .iter()
            .copied()
            .fold(f32::INFINITY, f32::min);
        let max_value = buffer
            .values
            .iter()
            .copied()
            .fold(f32::NEG_INFINITY, f32::max);
        let raw_span = (max_value - min_value).max(CHART_Y_MIN_SPAN);
        let y_padding = raw_span * CHART_Y_PADDING_RATIO;
        let y_min = min_value - y_padding;
        let y_max = max_value + y_padding;
        let y_span = (y_max - y_min).max(f32::EPSILON);

        let x_min = x_coords[0];
        let x_max = x_coords[x_coords.len() - 1];
        let x_span = (x_max - x_min).max(f32::EPSILON);

        let origin_x: f32 = bounds.origin.x.into();
        let origin_y: f32 = bounds.origin.y.into();
        let width: f32 = bounds.size.width.into();
        let height: f32 = bounds.size.height.into();
        let inset = 2.0;
        let plot_width = (width - inset * 2.0).max(1.0);
        let plot_height = (height - inset * 2.0).max(1.0);

        let grid_stroke = rgba(0x27272a);
        for grid_line in 1..4 {
            let grid_y = origin_y + inset + plot_height * grid_line as f32 / 4.0;
            let mut grid = PathBuilder::stroke(px(1.0));
            grid.move_to(point(px(origin_x + inset), px(grid_y)));
            grid.line_to(point(px(origin_x + inset + plot_width), px(grid_y)));
            if let Ok(path) = grid.build() {
                window.paint_path(path, grid_stroke);
            }
        }

        let mut builder = PathBuilder::stroke(px(CHART_STROKE_WIDTH));
        for (index, value) in buffer.values.iter().enumerate() {
            let t = (x_coords[index] - x_min) / x_span;
            let x = origin_x + inset + t * plot_width;
            let normalized = (*value - y_min) / y_span;
            let y = origin_y + inset + plot_height - normalized * plot_height;
            let chart_point = point(px(x), px(y));
            if index == 0 {
                builder.move_to(chart_point);
            } else {
                builder.line_to(chart_point);
            }
        }

        if let Ok(path) = builder.build() {
            window.paint_path(path, stroke);
        }
    }

    fn output_port_origin(
        node: &VisualNode,
        port_idx: usize,
        include_chart: bool,
        connections: &[NodeConnection],
    ) -> (f32, f32) {
        output_port_world_center(node, port_idx, include_chart, connections)
    }

    fn input_port_origin(
        node: &VisualNode,
        port_idx: usize,
        include_chart: bool,
        connections: &[NodeConnection],
    ) -> (f32, f32) {
        input_port_world_center(node, port_idx, include_chart, connections)
    }

    fn canvas_point(bounds: Bounds<Pixels>, x: f32, y: f32) -> Point<Pixels> {
        point(bounds.origin.x + px(x), bounds.origin.y + px(y))
    }

    fn paint_connection_wires(
        bounds: Bounds<Pixels>,
        nodes: &[VisualNode],
        connections: &[NodeConnection],
        chart_node_ids: &HashSet<usize>,
        active_wire_source: Option<(usize, usize)>,
        active_mouse_pos: Point<Pixels>,
        pan_offset: Point<Pixels>,
        zoom_scale: f32,
        window: &mut Window,
    ) {
        let pan_x: f32 = pan_offset.x.into();
        let pan_y: f32 = pan_offset.y.into();

        let world_to_screen = |world_x: f32, world_y: f32| -> (f32, f32) {
            (
                world_x * zoom_scale + pan_x,
                world_y * zoom_scale + pan_y,
            )
        };

        for connection in connections {
            let Some(from_node) = nodes
                .iter()
                .find(|node| node.id == connection.from_node_id)
            else {
                continue;
            };
            let Some(to_node) = nodes
                .iter()
                .find(|node| node.id == connection.to_node_id)
            else {
                continue;
            };

            let wire_kind = output_port_kind(from_node, connection.from_port_idx)
                .map(socket_kind_from_port)
                .unwrap_or(SocketWireKind::NumericSignal);
            let stroke = match wire_kind {
                SocketWireKind::StructuralPath => rgb(0x71717a),
                SocketWireKind::NumericSignal => rgb(0x22c55e),
                SocketWireKind::Aov => rgb(0x22d3ee),
            };

            let (out_x, out_y) = Self::output_port_origin(
                from_node,
                connection.from_port_idx,
                chart_node_ids.contains(&from_node.id),
                connections,
            );
            let (in_x, in_y) = Self::input_port_origin(
                to_node,
                connection.to_port_idx,
                chart_node_ids.contains(&to_node.id),
                connections,
            );
            let (screen_out_x, screen_out_y) = world_to_screen(out_x, out_y);
            let (screen_in_x, screen_in_y) = world_to_screen(in_x, in_y);
            let end = Self::canvas_point(bounds, screen_in_x, screen_in_y);
            Self::paint_bezier_wire(bounds, screen_out_x, screen_out_y, end, window, stroke);
        }

        if let Some((from_node_id, from_port_idx)) = active_wire_source {
            let Some(from_node) = nodes.iter().find(|node| node.id == from_node_id) else {
                return;
            };
            let (out_x, out_y) = Self::output_port_origin(
                from_node,
                from_port_idx,
                chart_node_ids.contains(&from_node.id),
                connections,
            );
            let (screen_out_x, screen_out_y) = world_to_screen(out_x, out_y);
            Self::paint_bezier_wire(
                bounds,
                screen_out_x,
                screen_out_y,
                active_mouse_pos,
                window,
                rgb(0x3b82f6),
            );
        }
    }

    fn paint_node_sockets(
        bounds: Bounds<Pixels>,
        nodes: &[VisualNode],
        connections: &[NodeConnection],
        chart_node_ids: &HashSet<usize>,
        pan_offset: Point<Pixels>,
        zoom_scale: f32,
        window: &mut Window,
    ) {
        let pan_x: f32 = pan_offset.x.into();
        let pan_y: f32 = pan_offset.y.into();
        let world_to_screen = |world_x: f32, world_y: f32| -> (f32, f32) {
            (
                world_x * zoom_scale + pan_x,
                world_y * zoom_scale + pan_y,
            )
        };

        for node in nodes {
            for (port_idx, _) in node.inputs.iter().enumerate() {
                if node.collapsed && !input_port_is_wired(node, port_idx, connections) {
                    continue;
                }
                let kind = input_port_kind(node, port_idx)
                    .map(socket_kind_from_port)
                    .unwrap_or(SocketWireKind::NumericSignal);
                let (world_x, world_y) = Self::input_port_origin(
                    node,
                    port_idx,
                    chart_node_ids.contains(&node.id),
                    connections,
                );
                let (screen_x, screen_y) = world_to_screen(world_x, world_y);
                paint_socket_dot(
                    window,
                    Self::canvas_point(bounds, screen_x, screen_y),
                    kind,
                );
            }
            for (port_idx, _) in node.outputs.iter().enumerate() {
                if node.collapsed && !output_port_is_wired(node, port_idx, connections) {
                    continue;
                }
                let kind = output_port_kind(node, port_idx)
                    .map(socket_kind_from_port)
                    .unwrap_or(SocketWireKind::NumericSignal);
                let (world_x, world_y) = Self::output_port_origin(
                    node,
                    port_idx,
                    chart_node_ids.contains(&node.id),
                    connections,
                );
                let (screen_x, screen_y) = world_to_screen(world_x, world_y);
                paint_socket_dot(
                    window,
                    Self::canvas_point(bounds, screen_x, screen_y),
                    kind,
                );
            }
        }
    }
    pub(crate) fn render_node_graph(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity().downgrade();
        let visible_nodes = self.canvas_visible_nodes();
        let visible_connections = self.canvas_visible_connections();
        let nodes_for_wires = visible_nodes.clone();
        let connections_for_wires = visible_connections.clone();
        let chart_node_ids: HashSet<usize> = self
            .asset_chart_history
            .iter()
            .filter(|(node_id, buffer)| {
                buffer.values.len() >= 2
                    && visible_nodes.iter().any(|node| node.id == **node_id)
            })
            .map(|(node_id, _)| *node_id)
            .collect();
        let chart_node_ids_for_wires = chart_node_ids.clone();
        let active_wire_source = self.active_wire_source;
        let active_mouse_pos = self.active_mouse_pos;
        let pan_offset = self.pan_offset;
        let zoom_scale = self.zoom_scale;

        let mut canvas = div()
            .on_children_prepainted({
                move |bounds: Vec<Bounds<Pixels>>, _window: &mut Window, cx: &mut App| {
                    if let Some(canvas_bounds) = union_canvas_bounds(&bounds) {
                        let origin = canvas_bounds.origin;
                        let _ = view.update(cx, |this, _cx| {
                            this.canvas_origin = origin;
                        });
                    }
                }
            })
            .id("node-canvas")
            .size_full()
            .min_h_0()
            .min_w_0()
            .bg(rgb(DCC_CANVAS_BACKPLATE))
            .relative()
            .overflow_hidden()
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, event: &MouseDownEvent, _window, cx| {
                    this.open_context_menu(event.position);
                    cx.notify();
                }),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _event: &MouseDownEvent, _window, cx| {
                    if this.context_menu_pos.is_some() {
                        this.dismiss_context_menu();
                    } else if this.active_wire_source.is_some() {
                        this.cancel_wire();
                    } else {
                        this.select_stage_path(None, cx);
                        this.ta_inspector_category = None;
                    }
                    cx.notify();
                }),
            )
            .on_mouse_move(cx.listener(
                |this, event: &MouseMoveEvent, _window, cx| {
                    let mut changed = false;
                    if this.is_panning {
                        this.update_pan(event.position);
                        changed = true;
                    }
                    if this.active_drag_node_id.is_some() {
                        this.update_dragged_node_position(event.position);
                        changed = true;
                    }
                    if this.active_wire_source.is_some() {
                        this.update_wire_tracking(event.position);
                        changed = true;
                    }
                    this.active_mouse_pos = event.position;
                    if changed {
                        cx.notify();
                    }
                },
            ))
            .on_mouse_down(
                MouseButton::Middle,
                cx.listener(|this, event: &MouseDownEvent, _window, cx| {
                    this.begin_pan(event.position);
                    cx.stop_propagation();
                    cx.notify();
                }),
            )
            .on_mouse_up(
                MouseButton::Middle,
                cx.listener(|this, _event: &MouseUpEvent, _window, cx| {
                    if this.is_panning {
                        this.end_pan();
                        cx.notify();
                    }
                }),
            )
            .on_mouse_up_out(
                MouseButton::Middle,
                cx.listener(|this, _event: &MouseUpEvent, _window, cx| {
                    if this.is_panning {
                        this.end_pan();
                        cx.notify();
                    }
                }),
            )
            .on_scroll_wheel(cx.listener(
                |this, event: &ScrollWheelEvent, _window, cx| {
                    this.apply_scroll_zoom(event);
                    cx.stop_propagation();
                    cx.notify();
                },
            ))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, event: &MouseUpEvent, _window, cx| {
                    this.handle_canvas_left_mouse_up(event.position, cx);
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, event: &MouseUpEvent, _window, cx| {
                    this.handle_canvas_left_mouse_up(event.position, cx);
                }),
            )
            .child(
                canvas(
                    |bounds, _window, _cx| bounds,
                    move |bounds, _state, window, _cx| {
                        paint_dcc_canvas_grid(bounds, pan_offset, zoom_scale, window);
                        TradingSystemWorkspace::paint_node_sockets(
                            bounds,
                            &nodes_for_wires,
                            &connections_for_wires,
                            &chart_node_ids_for_wires,
                            pan_offset,
                            zoom_scale,
                            window,
                        );
                        TradingSystemWorkspace::paint_connection_wires(
                            bounds,
                            &nodes_for_wires,
                            &connections_for_wires,
                            &chart_node_ids_for_wires,
                            active_wire_source,
                            active_mouse_pos,
                            pan_offset,
                            zoom_scale,
                            window,
                        );
                    },
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            );

        let suppress_charts = self.active_drag_node_id.is_some();
        let mut render_order: Vec<usize> = visible_nodes.iter().map(|node| node.id).collect();
        if let Some(drag_id) = self.active_drag_node_id {
            render_order.retain(|node_id| *node_id != drag_id);
            render_order.push(drag_id);
        }

        let view = cx.entity();
        let selected_path = self
            .workspace_context
            .read(cx)
            .selected_path()
            .map(str::to_string);

        for node_id in render_order {
            let Some(node) = visible_nodes.iter().find(|node| node.id == node_id) else {
                continue;
            };
            let node_prim_path = self.resolved_stage_path_for_node(node);
            let is_selected = node_prim_path
                .as_deref()
                .map(|path| selected_path.as_deref() == Some(path))
                .unwrap_or_else(|| self.selected_node_id == Some(node.id));
            let tier_accent = match &node.node_type {
                NodeType::TaUberSignal { config } => rgb(config.archetype.accent_rgb()),
                NodeType::OtlShader { .. } => rgb(0x9b87f5),
                NodeType::TerminalIntegrator { .. } => rgb(0x5eead4),
                NodeType::AssetAdaptor { .. } => rgb(0xd4a054),
            };
            let header_bg = match &node.node_type {
                NodeType::TaUberSignal { config } => rgb(ta_header_tint(config.archetype)),
                _ => rgb(DCC_HEADER_ACTIVE),
            };
            let hull_color = rgb(DCC_NODE_HULL);
            let border_color = if is_selected {
                rgb(NODE_SELECTION_HALO)
            } else {
                rgb(DCC_BORDER)
            };
            let (display_left, display_top) = self.world_to_screen(node.x, node.y);

            if node.collapsed {
                let pill_width = DCC_CAPSULE_WIDTH * self.zoom_scale;
                let pill_height = DCC_CAPSULE_HEIGHT * self.zoom_scale;
                canvas = canvas.child(
                    div()
                        .absolute()
                        .left(px(display_left))
                        .top(px(display_top))
                        .w(px(pill_width))
                        .h(px(pill_height))
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(hull_color)
                        .when(is_selected, |this| this.border_2())
                        .when(!is_selected, |this| this.border_1())
                        .border_color(border_color)
                        .when(node.node_type.is_ta_uber_signal(), |pill| {
                            pill.border_l_2().border_color(tier_accent)
                        })
                        .rounded_full()
                        .cursor_move()
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(
                                move |this, event: &MouseDownEvent, _window, cx| {
                                    if event.click_count >= 2 {
                                        this.toggle_node_collapsed(node_id, cx);
                                        cx.stop_propagation();
                                        return;
                                    }
                                    this.handle_aggregator_header_click(node_id, cx);
                                    this.begin_node_drag(node_id, event.position, cx);
                                    cx.stop_propagation();
                                    cx.notify();
                                },
                            ),
                        )
                        .child(
                            div()
                                .px_3()
                                .text_size(px(10.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(DCC_TEXT_PRIMARY))
                                .child(node.name.clone()),
                        ),
                );
                continue;
            }

            let display_width = NODE_WIDTH * self.zoom_scale;

            let mut ports = div().flex_col().gap_1().p_2();
            for (port_idx, input) in node.inputs.iter().enumerate() {
                let input_node_id = node_id;
                let has_wire = self.connections.iter().any(|connection| {
                    connection.to_node_id == input_node_id && connection.to_port_idx == port_idx
                });
                let input_kind = input_port_kind(node, port_idx)
                    .map(socket_kind_from_port)
                    .unwrap_or(SocketWireKind::NumericSignal);
                ports = ports.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .py_1()
                        .px_1()
                        .cursor_pointer()
                        .hover(|style| style.bg(rgb(DCC_NODE_SELECTED)))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(
                                move |this, _event: &MouseDownEvent, _window, cx| {
                                    if this.active_wire_source.is_some() {
                                        this.try_commit_wire_to_input(input_node_id, port_idx, cx);
                                    }
                                    cx.stop_propagation();
                                },
                            ),
                        )
                        .on_mouse_down(
                            MouseButton::Right,
                            cx.listener(
                                move |this, _event: &MouseDownEvent, _window, cx| {
                                    if has_wire {
                                        this.disconnect_input_wire(input_node_id, port_idx, cx);
                                        cx.stop_propagation();
                                    }
                                },
                            ),
                        )
                        .on_mouse_up(
                            MouseButton::Left,
                            cx.listener(
                                move |this, _event: &MouseUpEvent, _window, cx| {
                                    if this.active_wire_source.is_some() {
                                        this.try_commit_wire_to_input(input_node_id, port_idx, cx);
                                    }
                                    cx.stop_propagation();
                                },
                            ),
                        )
                        .text_size(px(9.0))
                        .font_family("monospace")
                        .text_color(rgb(0xaeaeb2))
                        .child(socket_pin(input_kind))
                        .child(format!("→ {input}")),
                );
            }
            for (port_idx, output) in node.outputs.iter().enumerate() {
                let output_node_id = node_id;
                let output_kind = output_port_kind(node, port_idx)
                    .map(socket_kind_from_port)
                    .unwrap_or(SocketWireKind::NumericSignal);
                ports = ports.child(
                    div()
                        .flex()
                        .items_center()
                        .justify_end()
                        .gap_1()
                        .py_1()
                        .px_1()
                        .cursor_pointer()
                        .hover(|style| style.bg(rgb(DCC_NODE_SELECTED)))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(
                                move |this, event: &MouseDownEvent, _window, cx| {
                                    this.begin_wire_from_output(
                                        output_node_id,
                                        port_idx,
                                        event.position,
                                    );
                                    cx.stop_propagation();
                                    cx.notify();
                                },
                            ),
                        )
                        .text_size(px(9.0))
                        .font_family("monospace")
                        .text_color(rgb(0xaeaeb2))
                        .child(format!("{output} →"))
                        .child(socket_pin(output_kind)),
                );
            }

            let mut node_card = div()
                .absolute()
                .left(px(display_left))
                .top(px(display_top))
                .w(px(display_width))
                .flex()
                .flex_col()
                .bg(hull_color)
                .when(is_selected, |this| this.border_2())
                .when(!is_selected, |this| this.border_1())
                .border_color(border_color)
                .rounded(px(DCC_NODE_CORNER_RADIUS_PX))
                .child(
                    div()
                        .id(("node-header", node_id))
                        .bg(header_bg)
                        .p_2()
                        .flex()
                        .items_center()
                        .justify_between()
                        .cursor_move()
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(
                                move |this, event: &MouseDownEvent, _window, cx| {
                                    if event.click_count >= 2 {
                                        this.toggle_node_collapsed(node_id, cx);
                                        cx.stop_propagation();
                                        return;
                                    }
                                    this.handle_aggregator_header_click(node_id, cx);
                                    this.begin_node_drag(node_id, event.position, cx);
                                    cx.stop_propagation();
                                    cx.notify();
                                },
                            ),
                        )
                        .child(
                            div()
                                .text_xs()
                                .font_weight(FontWeight::BOLD)
                                .text_color(rgb(DCC_TEXT_PRIMARY))
                                .child(node.name.clone()),
                        )
                        .child(div().w_2().h_2().bg(tier_accent).rounded_full()),
                )
                .child(
                    div()
                        .p_2()
                        .text_size(px(9.0))
                        .text_color(rgb(0xaeaeb2))
                        .child(format!("Grade Space: {:?}", node.grade)),
                );

            if let Some(config) = node.node_type.ta_uber_config() {
                node_card = node_card.child(
                    div()
                        .px_2()
                        .pb_2()
                        .text_size(px(8.0))
                        .font_family("monospace")
                        .text_color(rgb(0x8e8e93))
                        .child(archetype_summary(config)),
                );
            } else if node.node_type.is_otl_shader() {
                if let Some(script) = effective_otl_script(&node) {
                    let uniforms = parse_script_scalar_uniforms(script);
                    if !uniforms.is_empty() {
                        let mut param_panel = div().px_2().pb_2().flex_col().gap_1();
                        param_panel = param_panel.child(
                            div()
                                .text_size(px(8.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(0x8e8e93))
                                .child("Parameters"),
                        );
                        for param in uniforms {
                            let param_key = TradingSystemWorkspace::otl_shader_param_input_key(
                                node_id, &param.name,
                            );
                            let ty_label = match param.ty {
                                pulsar_marketlab_core::OslParamType::Int => "int",
                                pulsar_marketlab_core::OslParamType::Float => "float",
                                pulsar_marketlab_core::OslParamType::String => "string",
                            };
                            let mut row = div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .gap_2()
                                .child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .child(
                                            div()
                                                .text_size(px(8.0))
                                                .text_color(rgb(0xaeaeb2))
                                                .child(param.name.clone()),
                                        )
                                        .child(
                                            div()
                                                .text_size(px(7.0))
                                                .text_color(rgb(0x636366))
                                                .child(ty_label.to_string()),
                                        ),
                                );
                            if let Some(input) = self.otl_shader_param_inputs.get(&param_key) {
                                let integer = matches!(
                                    param.ty,
                                    pulsar_marketlab_core::OslParamType::Int
                                );
                                row = row.child(
                                    div()
                                        .flex_shrink_0()
                                        .w(px(112.0))
                                        .child(NodeNumberInput::new(input).integer(integer)),
                                );
                            }
                            param_panel = param_panel.child(row);
                        }
                        node_card = node_card.child(param_panel);
                    }
                }
            }

            if node.node_type.is_portfolio() {
                let wired_count = portfolio_wired_source_count(&self.connections, node_id);
                let allocation_id = node
                    .portfolio_allocation_id
                    .clone()
                    .unwrap_or_else(|| "Allocation::HierarchicalRiskParity".to_string());
                let allocation_label = PORTFOLIO_ALLOCATION_OPTIONS
                    .iter()
                    .find(|(token, _)| *token == allocation_id.as_str())
                    .map(|(_, label)| (*label).to_string())
                    .unwrap_or_else(|| allocation_id.clone());
                let prim_path = node_prim_path.clone().unwrap_or_default();
                let host_view = view.clone();
                node_card = node_card.child(
                    div()
                        .px_2()
                        .pb_2()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .text_size(px(8.0))
                                .text_color(rgb(0x8e8e93))
                                .child("Allocation"),
                        )
                        .child(
                            node_dropdown_trigger(
                                ("node-allocation", node_id),
                                allocation_label,
                                cx,
                            )
                            .dropdown_menu(move |menu, _, _| {
                                PORTFOLIO_ALLOCATION_OPTIONS
                                    .iter()
                                    .fold(menu, |menu, (token, label)| {
                                        let token = (*token).to_string();
                                        let prim_path = prim_path.clone();
                                        let view = host_view.clone();
                                        menu.item(
                                            PopupMenuItem::new(*label).on_click(move |_, _, cx| {
                                                let token = token.clone();
                                                let prim_path = prim_path.clone();
                                                let view_for_defer = view.clone();
                                                view.update(cx, |ws, cx| {
                                                    if let Some(node) = ws
                                                        .nodes
                                                        .iter_mut()
                                                        .find(|n| n.id == node_id)
                                                    {
                                                        node.portfolio_allocation_id =
                                                            Some(token.clone());
                                                    }
                                                    let workspace_context =
                                                        ws.workspace_context.clone();
                                                    cx.defer(move |cx| {
                                                        if !prim_path.is_empty() {
                                                            workspace_context.update(
                                                                cx,
                                                                |ctx, cx| {
                                                                    ctx.modify_attribute(
                                                                        &prim_path,
                                                                        "inputs:id",
                                                                        Value::String(
                                                                            token.clone(),
                                                                        ),
                                                                        cx,
                                                                    );
                                                                },
                                                            );
                                                        }
                                                        view_for_defer.update(cx, |ws, cx| {
                                                            ws.sync_pipeline_graph(cx);
                                                            cx.notify();
                                                        });
                                                    });
                                                    cx.notify();
                                                });
                                            }),
                                        )
                                    })
                            }),
                        ),
                );
                let node_metrics = cx
                    .global::<crate::ui::telemetry_bridge::MetricsTelemetryBridge>()
                    .metrics_for_node(node_id)
                    .cloned()
                    .or_else(|| {
                        self.portfolio_diagnostics_for_node(node_id)
                            .map(crate::ui::telemetry_bridge::EvaluatedMetrics::from)
                    });
                if let Some(metrics) = node_metrics {
                    let return_color = if metrics.total_return >= 0.0 {
                        rgb(0x10b981)
                    } else {
                        rgb(0xf87171)
                    };
                    node_card = node_card.child(
                        div()
                            .px_2()
                            .pb_1()
                            .flex_col()
                            .gap_0p5()
                            .font_family("monospace")
                            .text_size(px(8.0))
                            .child(
                                div()
                                    .text_color(rgb(0x64748b))
                                    .child(format!(
                                        "{wired_count} source(s) · {} trades · live GE",
                                        metrics.trailing_trades_count
                                    )),
                            )
                            .child(
                                div()
                                    .text_color(return_color)
                                    .child(format!(
                                        "R_total {}",
                                        format_percent_signed(metrics.total_return)
                                    )),
                            )
                            .child(
                                div()
                                    .text_color(rgb(0x94a3b8))
                                    .child(format!(
                                        "Exp {:.0}% · Conv {:.2}",
                                        metrics.net_exposure * 100.0,
                                        metrics.current_conviction
                                    )),
                            )
                            .child(
                                div()
                                    .text_color(rgb(0x64748b))
                                    .child(format!(
                                        "MDD {}",
                                        format_percent_signed(-metrics.rolling_drawdown)
                                    )),
                            ),
                    );
                } else if node_prim_path
                    .as_ref()
                    .and_then(|path| self.portfolio_timeline_cache.get(path))
                    .is_some_and(|series| !series.wealth.is_empty())
                {
                    let series = self
                        .portfolio_timeline_cache
                        .get(node_prim_path.as_ref().expect("prim path"))
                        .expect("series present");
                    let last_nav = series.wealth.last().copied().unwrap_or(0.0);
                    let base = series.wealth.first().copied().unwrap_or(last_nav);
                    let return_pct = if base.abs() > f64::EPSILON {
                        (last_nav / base - 1.0) * 100.0
                    } else {
                        0.0
                    };
                    let return_color = if return_pct >= 0.0 {
                        rgb(0x10b981)
                    } else {
                        rgb(0xf87171)
                    };
                    node_card = node_card.child(
                        div()
                            .px_2()
                            .pb_1()
                            .flex_col()
                            .gap_0p5()
                            .font_family("monospace")
                            .text_size(px(8.0))
                            .child(
                                div()
                                    .text_color(rgb(0x64748b))
                                    .child(format!(
                                        "{wired_count} source(s) · graph sweep · {} bars",
                                        series.wealth.len()
                                    )),
                            )
                            .child(
                                div()
                                    .text_color(return_color)
                                    .child(format!(
                                        "NAV {} · R {}",
                                        format_currency(last_nav),
                                        format_percent_signed(return_pct)
                                    )),
                            ),
                    );
                } else {
                    node_card = node_card.child(
                        div()
                            .px_2()
                            .pb_1()
                            .flex_col()
                            .gap_0p5()
                            .text_size(px(8.0))
                            .font_family("monospace")
                            .text_color(rgb(0x64748b))
                            .child(format!("{wired_count} execution source(s) wired"))
                            .child(self.portfolio_graph_engine_status_label(cx)),
                    );
                }
            }

            if node.node_type.displays_price_chart() && !suppress_charts {
                let chart_buffer = self
                    .asset_chart_history
                    .get(&node_id)
                    .cloned()
                    .unwrap_or_default();
                let chart_height = NODE_CHART_HEIGHT * self.zoom_scale;
                let chart_stroke = tier_accent;
                node_card = node_card.child(
                    div()
                        .px_2()
                        .pb_1()
                        .child(
                            gpui::canvas(
                                |bounds, _window, _cx| bounds,
                                move |bounds, _state, window, _cx| {
                                    TradingSystemWorkspace::paint_asset_price_chart(
                                        bounds,
                                        &chart_buffer,
                                        window,
                                        chart_stroke,
                                    );
                                },
                            )
                            .w_full()
                            .h(px(chart_height))
                            .rounded_sm()
                            .bg(rgb(DCC_CANVAS_BACKPLATE)),
                        ),
                );
            }

            canvas = canvas.child(node_card.child(ports));
        }

        if self.active_drag_node_id.is_some() {
            canvas = canvas.child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .on_mouse_move(cx.listener(
                        |this, event: &MouseMoveEvent, _window, cx| {
                            this.update_dragged_node_position(event.position);
                            cx.notify();
                        },
                    ))
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, _event: &MouseUpEvent, _window, cx| {
                            this.end_node_drag();
                            cx.notify();
                        }),
                    )
                    .on_mouse_up_out(
                        MouseButton::Left,
                        cx.listener(|this, _event: &MouseUpEvent, _window, cx| {
                            this.end_node_drag();
                            cx.notify();
                        }),
                    ),
            );
        }

        let view = cx.entity();
        canvas.context_menu({
            let view = view.clone();
            move |menu, _window, _cx| {
                let view = view.clone();
                let mut menu = menu
                    .item(
                        PopupMenuItem::new("Spawn Asset Node").on_click({
                            let view = view.clone();
                            move |_, _, cx| {
                                view.update(cx, |this, cx| {
                                    this.spawn_csv_asset_node(cx);
                                });
                            }
                        }),
                    )
                    .item(PopupMenuItem::separator())
                    .item(
                        PopupMenuItem::new(TaArchetype::Trend.spawn_menu_label()).on_click({
                            let view = view.clone();
                            move |_, _, cx| {
                                view.update(cx, |this, cx| {
                                    this.spawn_ta_uber_archetype(TaArchetype::Trend, cx);
                                });
                            }
                        }),
                    )
                    .item(
                        PopupMenuItem::new(TaArchetype::Volatility.spawn_menu_label()).on_click({
                            let view = view.clone();
                            move |_, _, cx| {
                                view.update(cx, |this, cx| {
                                    this.spawn_ta_uber_archetype(TaArchetype::Volatility, cx);
                                });
                            }
                        }),
                    )
                    .item(
                        PopupMenuItem::new(TaArchetype::Oscillator.spawn_menu_label()).on_click({
                            let view = view.clone();
                            move |_, _, cx| {
                                view.update(cx, |this, cx| {
                                    this.spawn_ta_uber_archetype(TaArchetype::Oscillator, cx);
                                });
                            }
                        }),
                    )
                    .item(
                        PopupMenuItem::new(TaArchetype::Channel.spawn_menu_label()).on_click({
                            let view = view.clone();
                            move |_, _, cx| {
                                view.update(cx, |this, cx| {
                                    this.spawn_ta_uber_archetype(TaArchetype::Channel, cx);
                                });
                            }
                        }),
                    )
                    .item(PopupMenuItem::separator());

                for preset in OTL_STDLIB_PRESETS {
                    let preset = *preset;
                    menu = menu.item(
                        PopupMenuItem::new(preset.menu_label).on_click({
                            let view = view.clone();
                            move |_, _, cx| {
                                view.update(cx, |this, cx| {
                                    this.spawn_otl_stdlib_preset(&preset, cx);
                                });
                            }
                        }),
                    );
                }

                menu.item(PopupMenuItem::separator())
                    .item(
                        PopupMenuItem::new("Spawn Portfolio Node").on_click({
                            let view = view.clone();
                            move |_, _, cx| {
                                view.update(cx, |this, cx| {
                                    this.spawn_portfolio_node(cx);
                                });
                            }
                        }),
                    )
            }
        })
    }
}
