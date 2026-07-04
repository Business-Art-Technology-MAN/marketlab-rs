//! MarketLab finance blueprint workstation binary (Track B Phase 1 host).
//!
//! Run from repo root:
//!   cargo run --manifest-path crates/marketlab_finance_editor/Cargo.toml

use blueprint_editor_plugin::{init_finance_otl_editor_keys, BlueprintEditorPanel, CompileMode};
use gpui::*;
use ui::{Assets, Root, Theme, ThemeMode};

fn main() {
    Application::new().with_assets(Assets).run(|cx: &mut App| {
        ui::init(cx);
        ui::themes::init(cx);
        init_finance_otl_editor_keys(cx);
        Theme::change(ThemeMode::Dark, None, cx);

        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Point {
                        x: px(60.0),
                        y: px(60.0),
                    },
                    size: Size {
                        width: px(1600.0),
                        height: px(960.0),
                    },
                })),
                titlebar: Some(TitlebarOptions {
                    title: Some("MarketLab Finance".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            move |window, cx| {
                let panel = cx.new(|cx| {
                    let mut panel = BlueprintEditorPanel::new(window, cx);
                    panel.compile_mode = CompileMode::MarketLabFinance;
                    panel
                });
                cx.new(|cx| Root::new(panel.into(), window, cx))
            },
        )
        .expect("failed to open MarketLab finance window");
    });
}
