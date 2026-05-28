//! Node canvas paint helpers: tier-colored socket dots and linkage wires.

use gpui::*;
use gpui::prelude::FluentBuilder;

/// Semantic socket coloring aligned with three-tier port wire kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SocketWireKind {
    /// Tier 1 structural / USD path reference.
    StructuralPath,
    /// Tier 2 numeric OTL signal.
    NumericSignal,
    /// Tier 2 arbitrary output variable (AOV).
    Aov,
}

pub fn socket_color(kind: SocketWireKind) -> Hsla {
    match kind {
        SocketWireKind::StructuralPath => rgb(0x71717a).into(),
        SocketWireKind::NumericSignal => rgb(0x22c55e).into(),
        SocketWireKind::Aov => rgb(0x22d3ee).into(),
    }
}

/// Render a DOM socket pin dot with the wire-kind color.
pub fn socket_pin(kind: SocketWireKind) -> impl IntoElement {
    div()
        .w(px(8.0))
        .h(px(8.0))
        .rounded_full()
        .bg(socket_color(kind))
        .border_1()
        .border_color(rgb(0x18181b))
}

/// Paint a filled socket circle on the canvas layer.
pub fn paint_socket_dot(window: &mut Window, center: Point<Pixels>, kind: SocketWireKind) {
    let radius = px(4.5);
    let diameter = radius * 2.0;
    window.paint_quad(fill(
        Bounds {
            origin: point(center.x - radius, center.y - radius),
            size: size(diameter, diameter),
        },
        socket_color(kind),
    ));
}

/// Paint a cubic bezier wire between two canvas points.
pub fn paint_bezier_wire(
    start: Point<Pixels>,
    end: Point<Pixels>,
    window: &mut Window,
    stroke: Hsla,
) {
    let start_x: f32 = start.x.into();
    let end_x: f32 = end.x.into();
    let mid_x = (start_x + end_x) / 2.0;
    let mut builder = PathBuilder::stroke(px(1.75));
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

/// Generic wire descriptor for painting active graph links.
#[derive(Clone, Debug)]
pub struct GraphWireSegment {
    pub from: Point<Pixels>,
    pub to: Point<Pixels>,
    pub kind: SocketWireKind,
}

pub fn paint_wires_for_graph(window: &mut Window, wires: &[GraphWireSegment]) {
    for wire in wires {
        let stroke = match wire.kind {
            SocketWireKind::StructuralPath => rgb(0x71717a).into(),
            SocketWireKind::NumericSignal => rgb(0x22c55e).into(),
            SocketWireKind::Aov => rgb(0x22d3ee).into(),
        };
        paint_bezier_wire(wire.from, wire.to, window, stroke);
    }
}

/// Red alert strip for graph compiler wiring validation failures.
pub fn render_wiring_alerts(messages: &[String]) -> impl IntoElement {
    let mut alerts = div().flex_col().gap_1();
    for message in messages {
        alerts = alerts.child(
            div()
                .px_3()
                .py_1()
                .bg(rgb(0x450a0a))
                .border_1()
                .border_color(rgb(0x991b1b))
                .text_size(px(10.0))
                .font_family("monospace")
                .text_color(rgb(0xfca5a5))
                .child(format!("⚠ wiring: {message}")),
        );
    }

    div().when(!messages.is_empty(), |this| {
        this.absolute()
            .top_10()
            .left_4()
            .right_4()
            .child(alerts)
    })
}
