//! Param Inspector pane: OTL script editor and AOV outbound toggles.

use gpui::*;
use gpui_component::checkbox::Checkbox;
use gpui_component::input::{Input, InputState};
use gpui_component::scroll::ScrollableElement;

pub trait ParamInspectorPane: Sized {
    fn param_inspector_title(&self) -> String;
    fn ensure_otl_script_input(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState>;
    fn otl_editing_enabled(&self) -> bool;
    fn aov_channel_options(&self) -> Vec<(String, bool)>;
    fn toggle_aov_channel(&mut self, channel: &str, enabled: bool, cx: &mut Context<Self>);
    fn render_param_inspector_extensions(
        &mut self,
        cx: &mut Context<Self>,
    ) -> AnyElement;
}

pub fn render_param_inspector<H: ParamInspectorPane + 'static>(
    view: Entity<H>,
    host: &mut H,
    window: &mut Window,
    cx: &mut Context<H>,
) -> impl IntoElement {
    let title = host.param_inspector_title();
    let otl_enabled = host.otl_editing_enabled();
    let aov_options = host.aov_channel_options();
    let otl_input = host.ensure_otl_script_input(window, cx);

    let mut body = div()
        .flex_1()
        .min_h_0()
        .overflow_y_scrollbar()
        .p_3()
        .flex_col()
        .gap_3()
        .child(
            div()
                .text_xs()
                .font_weight(FontWeight::BOLD)
                .text_color(rgb(0xe4e4e7))
                .child(title),
        );

    if otl_enabled {
        body = body
            .child(
                div()
                    .text_size(px(10.0))
                    .text_color(rgb(0x71717a))
                    .child("OTL Script"),
            )
            .child(
                Input::new(&otl_input)
                    .h(px(72.0))
                    .text_size(px(11.0))
                    .font_family("monospace"),
            );

        let mut aov_list = div().flex_col().gap_1();
        aov_list = aov_list.child(
            div()
                .text_size(px(10.0))
                .text_color(rgb(0x71717a))
                .child("AOV Outbound Pins"),
        );
        for (channel_index, (channel, enabled)) in aov_options.into_iter().enumerate() {
            let channel_id = channel.clone();
            aov_list = aov_list.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        Checkbox::new(("aov-channel", channel_index))
                            .checked(enabled)
                            .on_click({
                                let view = view.clone();
                                move |checked, window, cx| {
                                    let channel_id = channel_id.clone();
                                    view.update(cx, |host, cx| {
                                        host.toggle_aov_channel(&channel_id, *checked, cx);
                                    });
                                    let _ = window;
                                }
                            }),
                    )
                    .child(
                        div()
                            .text_xs()
                            .font_family("monospace")
                            .text_color(rgb(0x22d3ee))
                            .child(channel),
                    ),
            );
        }
        body = body.child(aov_list);
    }

    body = body.child(host.render_param_inspector_extensions(cx));

    div().size_full().flex().flex_col().child(body)
}
