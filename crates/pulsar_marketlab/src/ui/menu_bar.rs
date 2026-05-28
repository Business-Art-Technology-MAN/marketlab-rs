//! Menu bar trait bindings: native USD file IO.

use std::path::PathBuf;

use gpui::*;

use pulsar_marketlab_ui::workspace::MenuBarHost;
use pulsar_marketlab_core::FINANCIAL_SCHEMA_USDA;
use pulsar_marketlab::stage_bridge::usd_spike::UsdStageBridge;

use crate::workspace_state::TradingSystemWorkspace;

impl TradingSystemWorkspace {
    fn replace_usd_stage(&mut self, bridge: UsdStageBridge, cx: &mut Context<Self>) {
        self.usd_stage.update(cx, |stage, cx| {
            *stage = bridge;
            cx.notify();
        });
        self.invalidate_playhead_evaluation_cache();
        self.spawn_playhead_evaluation_async(cx);
        cx.notify();
    }

    fn finish_usd_save(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        match self.usd_stage.read(cx).dump_to_path(&path) {
            Ok(()) => {
                self.usd_document_path = Some(path.clone());
                self.push_status_log(format!(
                    "USD stage saved to `{}` // telemetry baseline OK",
                    path.display()
                ));
            }
            Err(error) => {
                self.push_status_log(format!(
                    "USD save failed for `{}`: {error}",
                    path.display()
                ));
            }
        }
        cx.notify();
    }
}

impl MenuBarHost for TradingSystemWorkspace {
    fn on_file_new(&mut self, cx: &mut Context<Self>) {
        self.reset_to_new_document(cx);
    }

    fn on_file_open(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let picked = cx
                .background_executor()
                .spawn(async {
                    rfd::AsyncFileDialog::new()
                        .add_filter("USD Layer", &["usda", "usd"])
                        .pick_file()
                        .await
                        .map(|handle| handle.path().to_path_buf())
                })
                .await;

            let Some(path) = picked else {
                return;
            };

            let path_for_error = path.clone();
            let loaded = cx
                .background_executor()
                .spawn(async move { UsdStageBridge::open(&path).map(|bridge| (bridge, path)) })
                .await;

            let _ = cx.update(|cx| {
                if let Some(entity) = this.upgrade() {
                    entity.update(cx, |workspace, cx| match loaded {
                        Ok((bridge, path)) => {
                            workspace.usd_document_path = Some(path.clone());
                            workspace.replace_usd_stage(bridge, cx);
                            workspace.push_status_log(format!(
                                "Opened USD stage `{}`",
                                path.display()
                            ));
                            cx.notify();
                        }
                        Err(error) => {
                            workspace.push_status_log(format!(
                                "USD open failed for `{}`: {error}",
                                path_for_error.display()
                            ));
                            cx.notify();
                        }
                    });
                }
            });
        })
        .detach();
    }

    fn on_file_save(&mut self, cx: &mut Context<Self>) {
        if let Some(path) = self.usd_document_path.clone() {
            self.finish_usd_save(path, cx);
            return;
        }
        self.on_file_save_as(cx);
    }

    fn on_file_save_as(&mut self, cx: &mut Context<Self>) {
        let default_name = self
            .usd_document_path
            .as_ref()
            .and_then(|path| path.file_name())
            .and_then(|name| name.to_str())
            .unwrap_or("marketlab_stage.usda")
            .to_string();

        cx.spawn(async move |this, cx| {
            let picked = cx
                .background_executor()
                .spawn(async {
                    rfd::AsyncFileDialog::new()
                        .set_file_name(default_name)
                        .add_filter("USD Layer", &["usda", "usd"])
                        .save_file()
                        .await
                        .map(|handle| handle.path().to_path_buf())
                })
                .await;

            let Some(path) = picked else {
                return;
            };

            let _ = cx.update(|cx| {
                if let Some(entity) = this.upgrade() {
                    entity.update(cx, |workspace, cx| {
                        workspace.finish_usd_save(path, cx);
                    });
                }
            });
        })
        .detach();
    }
}

#[allow(dead_code)]
pub(crate) fn financial_schema_usda() -> &'static str {
    FINANCIAL_SCHEMA_USDA
}
