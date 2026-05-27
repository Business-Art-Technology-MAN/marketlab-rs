//! Pipeline node canvas: dragging, wiring, context menus, and wire painting.

use std::collections::HashSet;

use gpui::*;

use crate::graph_compiler::{
    portfolio_signal_port_label, NodeConnection,
    portfolio_wired_source_count,
    connection_is_valid, input_port_world_center, node_shows_price_chart,
    output_port_world_center, portfolio_ensure_spare_input_port, portfolio_resolve_input_port,
    NodeGradeType, NodeType, VisualNode, CONNECTION_STROKE_WIDTH, MAX_ZOOM, MIN_ZOOM,
    NODE_CHART_HEIGHT, NODE_SPAWN_STAGGER_X, NODE_SPAWN_STAGGER_Y, NODE_WIDTH,
    WIRE_PORT_HIT_RADIUS, ZOOM_WHEEL_SENSITIVITY,
};
use pulsar_marketlab::technical_analysis::{ta_indicator_label, DEFAULT_TA_INDICATOR_ID, DEFAULT_TA_LOOKBACK};
use crate::workspace_state::{
    parse_chart_date_ordinal, ChartHistoryBuffer, CHART_Y_MIN_SPAN, CHART_Y_PADDING_RATIO,
    format_percent_signed, format_ratio,
    TaExecutionBridge, TradingSystemWorkspace, CHART_STROKE_WIDTH,
};

impl TradingSystemWorkspace {
    fn canvas_local_position(&self, position: Point<Pixels>) -> (f32, f32) {
        let mouse_x: f32 = position.x.into();
        let mouse_y: f32 = position.y.into();
        let canvas_x: f32 = self.canvas_origin.x.into();
        let canvas_y: f32 = self.canvas_origin.y.into();
        (mouse_x - canvas_x, mouse_y - canvas_y)
    }

    fn screen_to_world(&self, local_x: f32, local_y: f32) -> (f32, f32) {
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
            self.selected_node_id = Some(node_id);
            self.sync_inspector_from_selection(cx);
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
    fn next_node_id(&self) -> usize {
        self.nodes.iter().map(|node| node.id).max().unwrap_or(0) + 1
    }

    fn spawn_technical_analysis_node(&mut self, cx: &mut Context<Self>) {
        let Some(menu_pos) = self.context_menu_pos else {
            return;
        };

        let x: f32 = menu_pos.x.into();
        let y: f32 = menu_pos.y.into();
        let node_id = self.next_node_id();
        let ta_index = self
            .nodes
            .iter()
            .filter(|node| node.node_type == NodeType::TechnicalAnalysis)
            .count();

        self.nodes.push(VisualNode {
            id: node_id,
            name: ta_indicator_label(DEFAULT_TA_INDICATOR_ID)
                .unwrap_or("RSI")
                .to_string(),
            node_type: NodeType::TechnicalAnalysis,
            grade: NodeGradeType::Scalar,
            ta_indicator_id: Some(DEFAULT_TA_INDICATOR_ID.to_string()),
            ta_lookback_period: DEFAULT_TA_LOOKBACK as u32,
            dsl_formula: None,
            asset_source: None,
            x: x + ta_index as f32 * NODE_SPAWN_STAGGER_X,
            y: y + ta_index as f32 * NODE_SPAWN_STAGGER_Y,
            inputs: vec!["Price In".to_string()],
            outputs: vec!["TA Out".to_string()],
        });
        self.selected_node_id = Some(node_id);
        self.sync_ta_inspector_category_from_selection();
        self.context_menu_pos = None;
        self.sync_pipeline_graph();
        self.invalidate_playhead_evaluation_cache();
        cx.notify();
    }

    fn spawn_csv_asset_node(&mut self, cx: &mut Context<Self>) {
        let Some(menu_pos) = self.context_menu_pos else {
            return;
        };

        let x: f32 = menu_pos.x.into();
        let y: f32 = menu_pos.y.into();
        let node_id = self.next_node_id();
        let asset_index = self
            .nodes
            .iter()
            .filter(|node| node.node_type == NodeType::Asset)
            .count();

        self.nodes.push(VisualNode {
            id: node_id,
            name: "CSV Asset".to_string(),
            node_type: NodeType::Asset,
            grade: NodeGradeType::Scalar,
            ta_indicator_id: None,
            ta_lookback_period: DEFAULT_TA_LOOKBACK as u32,
            dsl_formula: None,
            asset_source: None,
            x: x + asset_index as f32 * NODE_SPAWN_STAGGER_X,
            y: y + asset_index as f32 * NODE_SPAWN_STAGGER_Y,
            inputs: vec![],
            outputs: vec!["Close Out".to_string()],
        });
        self.selected_node_id = Some(node_id);
        self.asset_path_input.update(cx, |input, cx| {
            input.set_content(String::new(), cx);
        });
        self.context_menu_pos = None;
        self.sync_pipeline_graph();
        self.invalidate_playhead_evaluation_cache();
        self.prompt_csv_for_node(node_id, cx);
    }

    fn spawn_portfolio_node(&mut self, cx: &mut Context<Self>) {
        let Some(menu_pos) = self.context_menu_pos else {
            return;
        };

        let x: f32 = menu_pos.x.into();
        let y: f32 = menu_pos.y.into();
        let node_id = self.next_node_id();
        let portfolio_index = self
            .nodes
            .iter()
            .filter(|node| node.node_type == NodeType::Portfolio)
            .count();

        self.nodes.push(VisualNode {
            id: node_id,
            name: format!("Sim Portfolio {}", portfolio_index + 1),
            node_type: NodeType::Portfolio,
            grade: NodeGradeType::Scalar,
            ta_indicator_id: None,
            ta_lookback_period: DEFAULT_TA_LOOKBACK as u32,
            dsl_formula: None,
            asset_source: None,
            x: x + portfolio_index as f32 * NODE_SPAWN_STAGGER_X,
            y: y + portfolio_index as f32 * NODE_SPAWN_STAGGER_Y,
            inputs: vec![portfolio_signal_port_label(0)],
            outputs: vec!["NAV Out".to_string()],
        });
        self.selected_node_id = Some(node_id);
        self.context_menu_pos = None;
        self.sync_pipeline_graph();
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
        let from_is_ta = self
            .nodes
            .iter()
            .find(|node| node.id == from_node_id)
            .is_some_and(|node| node.node_type == NodeType::TechnicalAnalysis);

        self.connections.retain(|connection| {
            !(connection.to_node_id == to_node_id && connection.to_port_idx == to_port_idx)
        });

        if from_is_ta {
            let mut bridge = TaExecutionBridge::new();
            bridge.clear_ta_signal_slot(from_node_id, &mut self.market_stage);
        }

        self.push_status_log(format!(
            "Wire disconnected — node {from_node_id} → node {to_node_id} port {to_port_idx}"
        ));
        self.sync_pipeline_graph();
        self.invalidate_playhead_evaluation_cache();
        self.recompute_playhead_diagnostics();
        cx.notify();
    }

    fn commit_wire_to_input(&mut self, to_node_id: usize, to_port_idx: usize) -> Option<(usize, usize)> {
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

        let effective_port = if to_node.node_type == NodeType::Portfolio {
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
        if to_node.node_type == NodeType::Portfolio {
            portfolio_ensure_spare_input_port(&mut self.nodes, &self.connections, to_node_id);
        }
        self.active_wire_source = None;
        self.sync_pipeline_graph();
        Some((effective_port, from_node_id))
    }

    fn try_commit_wire_to_input(&mut self, to_node_id: usize, to_port_idx: usize, cx: &mut Context<Self>) {
        if let Some((port, from_node_id)) = self.commit_wire_to_input(to_node_id, to_port_idx) {
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

        for node in &self.nodes {
            let include_chart = self.node_includes_chart(node);
            for (port_idx, _) in node.inputs.iter().enumerate() {
                let (port_world_x, port_world_y) =
                    input_port_world_center(node, port_idx, include_chart);
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
        );
        let (port_screen_x, port_screen_y) = self.world_to_screen(port_world_x, port_world_y);
        let dx = local_x - port_screen_x;
        let dy = local_y - port_screen_y;
        dx * dx + dy * dy <= WIRE_PORT_HIT_RADIUS * WIRE_PORT_HIT_RADIUS
    }

    fn handle_canvas_left_mouse_up(&mut self, screen_pos: Point<Pixels>, cx: &mut Context<Self>) {
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

    fn output_port_origin(node: &VisualNode, port_idx: usize, include_chart: bool) -> (f32, f32) {
        output_port_world_center(node, port_idx, include_chart)
    }

    fn input_port_origin(node: &VisualNode, port_idx: usize, include_chart: bool) -> (f32, f32) {
        input_port_world_center(node, port_idx, include_chart)
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
        let stroke = rgb(0x3b82f6);
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

            let (out_x, out_y) =
                Self::output_port_origin(from_node, connection.from_port_idx, chart_node_ids.contains(&from_node.id));
            let (in_x, in_y) = Self::input_port_origin(to_node, connection.to_port_idx, chart_node_ids.contains(&to_node.id));
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
            );
            let (screen_out_x, screen_out_y) = world_to_screen(out_x, out_y);
            Self::paint_bezier_wire(
                bounds,
                screen_out_x,
                screen_out_y,
                active_mouse_pos,
                window,
                stroke,
            );
        }
    }
    pub(crate) fn render_node_graph(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity().downgrade();
        let nodes_for_wires = self.nodes.clone();
        let connections_for_wires = self.connections.clone();
        let chart_node_ids: HashSet<usize> = self
            .asset_chart_history
            .iter()
            .filter(|(_, buffer)| buffer.values.len() >= 2)
            .map(|(node_id, _)| *node_id)
            .collect();
        let chart_node_ids_for_wires = chart_node_ids.clone();
        let active_wire_source = self.active_wire_source;
        let active_mouse_pos = self.active_mouse_pos;
        let pan_offset = self.pan_offset;
        let zoom_scale = self.zoom_scale;

        let mut canvas = div()
            .flex_1()
            .min_h_0()
            .bg(rgb(0x111114))
            .relative()
            .overflow_hidden()
            .on_children_prepainted({
                move |bounds: Vec<Bounds<Pixels>>, _window: &mut Window, cx: &mut App| {
                    if let Some(canvas_bounds) = bounds.first() {
                        let origin = canvas_bounds.origin;
                        view
                            .update(cx, |this, _cx| {
                                this.canvas_origin = origin;
                            })
                            .ok();
                    }
                }
            })
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(|this, event: &MouseDownEvent, _window, cx| {
                            this.open_context_menu(event.position);
                            cx.stop_propagation();
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
                                this.selected_node_id = None;
                                this.ta_inspector_category = None;
                            }
                            cx.notify();
                        }),
                    ),
            )
            .child(
                canvas(
                    |bounds, _window, _cx| bounds,
                    move |bounds, _state, window, _cx| {
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
                div()
                    .absolute()
                    .top_3()
                    .left_4()
                    .text_size(px(10.0))
                    .font_family("monospace")
                    .text_color(rgb(0x3b82f6))
                    .child("░ MarketLab Pipeline Node Canvas // Active"),
            );

        let suppress_charts = self.active_drag_node_id.is_some();
        let mut render_order: Vec<usize> = self.nodes.iter().map(|node| node.id).collect();
        if let Some(drag_id) = self.active_drag_node_id {
            render_order.retain(|node_id| *node_id != drag_id);
            render_order.push(drag_id);
        }

        for node_id in render_order {
            let Some(node) = self.nodes.iter().find(|node| node.id == node_id) else {
                continue;
            };
            let color = match node.node_type {
                NodeType::TechnicalAnalysis => rgb(0xa855f7),
                NodeType::Portfolio => rgb(0x14b8a6),
                _ => match node.grade {
                    NodeGradeType::Scalar => rgb(0xf59e0b),
                    NodeGradeType::Vector => rgb(0x3b82f6),
                    NodeGradeType::Trivector => rgb(0x8b5cf6),
                },
            };
            let border_color = if self.selected_node_id == Some(node.id) {
                rgb(0x3b82f6)
            } else {
                rgb(0x2d2d34)
            };
            let (display_left, display_top) = self.world_to_screen(node.x, node.y);
            let display_width = NODE_WIDTH * self.zoom_scale;

            let mut ports = div().flex_col().gap_1().p_2();
            for (port_idx, input) in node.inputs.iter().enumerate() {
                let input_node_id = node_id;
                let has_wire = self.connections.iter().any(|connection| {
                    connection.to_node_id == input_node_id && connection.to_port_idx == port_idx
                });
                ports = ports.child(
                    div()
                        .flex()
                        .items_center()
                        .py_1()
                        .px_1()
                        .cursor_pointer()
                        .hover(|style| style.bg(rgb(0x25252b)))
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
                        .text_color(rgb(0x888893))
                        .child(format!("→ [ ] {input}")),
                );
            }
            for (port_idx, output) in node.outputs.iter().enumerate() {
                let output_node_id = node_id;
                ports = ports.child(
                    div()
                        .flex()
                        .justify_end()
                        .py_1()
                        .px_1()
                        .cursor_pointer()
                        .hover(|style| style.bg(rgb(0x25252b)))
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
                        .text_color(rgb(0x888893))
                        .child(format!("[ ] → {output}")),
                );
            }

            let mut node_card = div()
                .absolute()
                .left(px(display_left))
                .top(px(display_top))
                .w(px(display_width))
                .flex()
                .flex_col()
                .bg(rgb(0x1c1c21))
                .border_1()
                .border_color(border_color)
                .rounded_md()
                .child(
                    div()
                        .id(("node-header", node_id))
                        .bg(rgb(0x25252b))
                        .p_2()
                        .flex()
                        .items_center()
                        .justify_between()
                        .cursor_move()
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(
                                move |this, event: &MouseDownEvent, _window, cx| {
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
                                .text_color(rgb(0xffffff))
                                .child(node.name.clone()),
                        )
                        .child(div().w_2().h_2().bg(color).rounded_full()),
                )
                .child(
                    div()
                        .p_2()
                        .text_size(px(9.0))
                        .text_color(rgb(0x888893))
                        .child(format!("Grade Space: {:?}", node.grade)),
                );

            if node.node_type == NodeType::TechnicalAnalysis {
                let indicator_label = node
                    .ta_indicator_id
                    .as_deref()
                    .and_then(ta_indicator_label)
                    .or_else(|| node.ta_indicator_id.as_deref())
                    .unwrap_or("unbound");
                node_card = node_card.child(
                    div()
                        .px_2()
                        .pb_1()
                        .text_size(px(8.0))
                        .font_family("monospace")
                        .text_color(rgb(0xc084fc))
                        .child(format!("VectorTA // {indicator_label}")),
                );
            }

            if node.node_type == NodeType::Portfolio {
                let wired_count = portfolio_wired_source_count(&self.connections, node_id);
                if let Some(metrics) = &self.portfolio_diagnostics {
                    let return_color = if metrics.total_return_pct >= 0.0 {
                        rgb(0x10b981)
                    } else {
                        rgb(0xf87171)
                    };
                    let alpha_color = metrics
                        .excess_return_pct
                        .map(|alpha| {
                            if alpha >= 0.0 {
                                rgb(0x10b981)
                            } else {
                                rgb(0xf87171)
                            }
                        })
                        .unwrap_or(rgb(0x64748b));
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
                                        "{wired_count} source(s) · {} trades · {} bars",
                                        metrics.trade_count, metrics.bars_processed
                                    )),
                            )
                            .child(
                                div()
                                    .text_color(return_color)
                                    .child(format!(
                                        "R_total {}",
                                        format_percent_signed(metrics.total_return_pct)
                                    )),
                            )
                            .child(
                                div()
                                    .text_color(alpha_color)
                                    .child(format!(
                                        "α vs B&H {}",
                                        metrics
                                            .excess_return_pct
                                            .map(format_percent_signed)
                                            .unwrap_or_else(|| "—".to_string())
                                    )),
                            )
                            .child(
                                div()
                                    .text_color(rgb(0x94a3b8))
                                    .child(format!(
                                        "Exp {:.0}% · Sharpe {}",
                                        metrics.avg_exposure_pct * 100.0,
                                        format_ratio(metrics.sharpe_ratio)
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
                            .child("Awaiting ledger sync…"),
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
                let chart_stroke = color;
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
                            .bg(rgb(0x141417)),
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

        if let Some(menu_pos) = self.context_menu_pos {
            let menu_x: f32 = menu_pos.x.into();
            let menu_y: f32 = menu_pos.y.into();
            let (menu_screen_x, menu_screen_y) = self.world_to_screen(menu_x, menu_y);
            canvas = canvas.child(
                div()
                    .absolute()
                    .left(px(menu_screen_x))
                    .top(px(menu_screen_y))
                    .bg(rgb(0x1c1c21))
                    .border_1()
                    .border_color(rgb(0x2d2d34))
                    .rounded_md()
                    .p_1()
                    .flex_col()
                    .gap_0p5()
                    .child(
                        div()
                            .px_2()
                            .py_1()
                            .text_size(px(9.0))
                            .font_family("monospace")
                            .text_color(rgb(0xf59e0b))
                            .cursor_pointer()
                            .hover(|style| style.bg(rgb(0x25252b)))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _event: &MouseDownEvent, _window, cx| {
                                    this.spawn_csv_asset_node(cx);
                                    cx.stop_propagation();
                                    cx.notify();
                                }),
                            )
                            .child("Spawn Asset Node"),
                    )
                    .child(
                        div()
                            .px_2()
                            .py_1()
                            .text_size(px(9.0))
                            .font_family("monospace")
                            .text_color(rgb(0xc084fc))
                            .cursor_pointer()
                            .hover(|style| style.bg(rgb(0x25252b)))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _event: &MouseDownEvent, _window, cx| {
                                    this.spawn_technical_analysis_node(cx);
                                    cx.stop_propagation();
                                    cx.notify();
                                }),
                            )
                            .child("Spawn TA Node"),
                    )
                    .child(
                        div()
                            .px_2()
                            .py_1()
                            .text_size(px(9.0))
                            .font_family("monospace")
                            .text_color(rgb(0x14b8a6))
                            .cursor_pointer()
                            .hover(|style| style.bg(rgb(0x25252b)))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _event: &MouseDownEvent, _window, cx| {
                                    this.spawn_portfolio_node(cx);
                                    cx.stop_propagation();
                                    cx.notify();
                                }),
                            )
                            .child("Spawn Portfolio Node"),
                    ),
            );
        }

        canvas
    }
}
