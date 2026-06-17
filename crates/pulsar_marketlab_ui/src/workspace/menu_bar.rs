//! Global workstation menu bar with File actions.

use gpui::*;
use gpui_component::button::DropdownButton;
use gpui_component::menu::{PopupMenu, PopupMenuItem};

use crate::theme;
use crate::theme::buttons::dcc_toolbar_button;

/// Host callbacks for menu bar file operations.
pub trait MenuBarHost: Sized {
    fn on_file_new(&mut self, cx: &mut Context<Self>);
    fn on_file_open(&mut self, cx: &mut Context<Self>);
    fn on_file_save(&mut self, cx: &mut Context<Self>);
    fn on_file_save_as(&mut self, cx: &mut Context<Self>);
}

/// Top-edge GPUI menu bar view.
pub struct MenuBar;

impl MenuBar {
    pub fn render<H: MenuBarHost + 'static>(
        view: Entity<H>,
        _host: &H,
        _window: &mut Window,
        cx: &mut Context<H>,
    ) -> impl IntoElement {
        let file_menu = move |menu: PopupMenu, _window: &mut Window, _cx: &mut Context<PopupMenu>| {
            let view = view.clone();
            menu.item(PopupMenuItem::new("New").on_click({
                let view = view.clone();
                move |_, window, cx| {
                    let view = view.clone();
                    window.defer(cx, move |_, cx| {
                        view.update(cx, |host, cx| host.on_file_new(cx));
                    });
                }
            }))
            .item(PopupMenuItem::new("Open").on_click({
                let view = view.clone();
                move |_, window, cx| {
                    let view = view.clone();
                    window.defer(cx, move |_, cx| {
                        view.update(cx, |host, cx| host.on_file_open(cx));
                    });
                }
            }))
            .item(PopupMenuItem::separator())
            .item(PopupMenuItem::new("Save").on_click({
                let view = view.clone();
                move |_, window, cx| {
                    let view = view.clone();
                    window.defer(cx, move |_, cx| {
                        view.update(cx, |host, cx| host.on_file_save(cx));
                    });
                }
            }))
            .item(PopupMenuItem::new("Save As…").on_click({
                let view = view.clone();
                move |_, window, cx| {
                    let view = view.clone();
                    window.defer(cx, move |_, cx| {
                        view.update(cx, |host, cx| host.on_file_save_as(cx));
                    });
                }
            }))
        };

        div()
            .id("marketlab-menu-bar")
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .py_1()
            .bg(rgb(theme::TOOLBAR_BG))
            .border_b_1()
            .border_color(rgb(theme::TOOLBAR_BORDER))
            .child(
                DropdownButton::new("menu-file")
                    .compact()
                    .button(dcc_toolbar_button("menu-file-trigger", "File", cx))
                    .dropdown_menu(file_menu),
            )
            .child(
                div()
                    .text_size(px(10.0))
                    .font_family("monospace")
                    .text_color(rgb(theme::TEXT_HINT))
                    .child("Ctrl+N New · Ctrl+O Open · Ctrl+S Save"),
            )
    }
}
