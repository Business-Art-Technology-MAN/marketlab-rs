//! GPUI shell layout.

mod node_canvas;
mod sidebar_inspector;
mod timeline_controls;

use gpui::*;

use crate::workspace_state::TradingSystemWorkspace;

impl Render for TradingSystemWorkspace {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0x09090b))
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_row()
                    .min_h_0()
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .min_h_0()
                            .min_w_0()
                            .child(self.render_ohlc_chart_pane(cx))
                            .child(self.render_node_graph(cx)),
                    )
                    .child(
                        div()
                            .w(px(384.0))
                            .flex_shrink_0()
                            .h_full()
                            .min_h_0()
                            .overflow_hidden()
                            .bg(rgb(0x0c0c0e))
                            .border_l_1()
                            .border_color(rgb(0x222227))
                            .flex()
                            .flex_col()
                            .child(self.render_spreadsheet_inspector(cx)),
                    ),
            )
            .child(self.render_bottom_metrics_panel())
    }
}
