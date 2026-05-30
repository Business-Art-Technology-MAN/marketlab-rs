//! Menu bar trait bindings: native USD file IO.

use std::path::PathBuf;

use gpui::*;

use pulsar_marketlab_ui::workspace::{MenuBarHost, WorkspaceContext};
use pulsar_marketlab::stage_bridge::usd_spike::UsdStageBridge;

use crate::canvas_compose::write_pipeline_usd_document;
use crate::canvas_hydrate::hydrate_canvas_from_stage;
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
        match write_pipeline_usd_document(&path, &self.nodes, &self.connections) {
            Ok(()) => match UsdStageBridge::open(&path) {
                Ok(bridge) => {
                    self.usd_document_path = Some(path.clone());
                    self.replace_usd_stage(bridge, cx);
                    if let Ok(context) = WorkspaceContext::new(&path) {
                        self.workspace_context.update(cx, |ctx, cx| {
                            *ctx = context;
                            cx.notify();
                        });
                    }
                    self.push_status_log(format!(
                        "USD stage saved to `{}` (schema sidecar co-located)",
                        path.display()
                    ));
                }
                Err(error) => {
                    self.push_status_log(format!(
                        "USD saved but reload failed for `{}`: {error}",
                        path.display()
                    ));
                }
            },
            Err(error) => {
                self.push_status_log(format!(
                    "USD save failed for `{}`: {error}",
                    path.display()
                ));
            }
        }
        cx.notify();
    }

    pub(crate) fn load_usd_document(
        &mut self,
        bridge: UsdStageBridge,
        path: PathBuf,
        cx: &mut Context<Self>,
    ) {
        let hydrated = hydrate_canvas_from_stage(&bridge);
        self.nodes = hydrated.nodes;
        self.connections = hydrated.connections;
        self.selected_node_id = None;
        self.active_drag_node_id = None;
        self.active_wire_source = None;
        self.context_menu_pos = None;
        self.usd_document_path = Some(path.clone());
        self.node_lookback_inputs.clear();
        self.node_lookback_inputs_ready = false;
        self.csv_path_registry
            .replace_from_nodes(&self.nodes);
        self.pipeline_graph
            .replace(self.nodes.clone(), self.connections.clone());

        self.usd_stage.update(cx, |stage, cx| {
            *stage = bridge;
            cx.notify();
        });

        if let Ok(context) = WorkspaceContext::new(&path) {
            self.workspace_context.update(cx, |ctx, cx| {
                *ctx = context;
                cx.notify();
            });
        } else {
            self.push_status_log(format!(
                "USD opened `{}` but workspace context reload failed; stage tree may be stale until save.",
                path.display()
            ));
        }

        self.sync_pipeline_graph(cx);
        self.sync_playhead_bounds();
        self.synchronize_inspector_view();
        self.recompute_playhead_diagnostics();
        self.invalidate_playhead_evaluation_cache();
        self.spawn_playhead_evaluation_async(cx);
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
                            workspace.load_usd_document(bridge, path.clone(), cx);
                            workspace.push_status_log(format!(
                                "Opened USD stage `{}` ({} nodes hydrated)",
                                path.display(),
                                workspace.nodes.len()
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

