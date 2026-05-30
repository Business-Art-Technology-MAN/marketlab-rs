//! Spreadsheet inspector sidebar: parameters, ledgers, and analytics panels.

use gpui::*;
use gpui::prelude::FluentBuilder;
use gpui_component::scroll::ScrollableElement;

use crate::asset_path_input::render_asset_path_input;
use crate::graph_compiler::{AssetSourceType, NodeType, VisualNode};
use pulsar_marketlab::technical_analysis::{
    category_display_label, ta_category_for_indicator, ta_indicator_catalog_hierarchy,
    ta_indicator_label, TA_SIDEBAR_ALGORITHMS, DEFAULT_TA_INDICATOR_ID, DEFAULT_TA_LOOKBACK, MAX_TA_LOOKBACK, MIN_TA_LOOKBACK, clamp_ta_lookback,
};
use std::path::PathBuf;
use crate::asset_path_input::PathInputEvent;
use crate::workspace_state::{
    chart_buffer_from_csv_rows, csv_node_label_from_path, hydrate_market_stage_from_ohlc,
    load_yahoo_finance_csv, ohlc_bars_from_csv_rows, format_currency, format_percent_magnitude,
    format_percent_signed, format_ratio, format_tick_label, MatrixDataRow, TradingSystemWorkspace,
    SIM_INITIAL_CASH,
};

impl TradingSystemWorkspace {
    pub(crate) fn selected_technical_analysis_node(&self) -> Option<&VisualNode> {
        let selected_id = self.selected_node_id?;
        self.nodes.iter().find(|node| {
            node.id == selected_id && node.node_type.is_otl_shader()
        })
    }

    pub(crate) fn selected_portfolio_node(&self) -> Option<&VisualNode> {
        let selected_id = self.selected_node_id?;
        self.nodes.iter().find(|node| {
            node.id == selected_id && node.node_type.is_portfolio()
        })
    }

    pub(crate) fn set_ta_indicator(&mut self, node_id: usize, indicator_id: String, cx: &mut Context<Self>) {
        let label = ta_indicator_label(&indicator_id)
            .map(str::to_string)
            .unwrap_or_else(|| indicator_id.clone());
        if let Some(node) = self
            .nodes
            .iter_mut()
            .find(|node| node.id == node_id && node.node_type.is_otl_shader())
        {
            node.ta_indicator_id = Some(indicator_id.clone());
            node.name = label;
            self.ta_inspector_category = ta_category_for_indicator(&indicator_id);
            self.sync_pipeline_graph(cx);
            self.invalidate_playhead_evaluation_cache();
            self.recompute_playhead_diagnostics();
            cx.notify();
        }
    }

    pub(crate) fn commit_ta_parameter_change(&mut self, cx: &mut Context<Self>) {
        self.sync_pipeline_graph(cx);
        self.invalidate_playhead_evaluation_cache();
        self.recompute_playhead_diagnostics();
        cx.notify();
    }

    pub(crate) fn set_ta_lookback_period(&mut self, node_id: usize, period: u32) {
        let period = clamp_ta_lookback(period as usize) as u32;
        if let Some(node) = self.nodes.iter_mut().find(|node| {
            node.id == node_id && node.node_type.is_otl_shader()
        }) {
            node.ta_lookback_period = period;
        }
    }

    pub(crate) fn lookback_from_slider_position(&self, mouse_x: f32, bounds: Bounds<Pixels>) -> u32 {
        let origin_x: f32 = bounds.origin.x.into();
        let width: f32 = bounds.size.width.into();
        if width <= f32::EPSILON {
            return DEFAULT_TA_LOOKBACK as u32;
        }
        let t = ((mouse_x - origin_x) / width).clamp(0.0, 1.0);
        let span = (MAX_TA_LOOKBACK - MIN_TA_LOOKBACK) as f32;
        clamp_ta_lookback(MIN_TA_LOOKBACK + (t * span).round() as usize) as u32
    }

    pub(crate) fn begin_ta_lookback_scrub(&mut self, node_id: usize, mouse_x: f32, cx: &mut Context<Self>) {
        let Some(bounds) = self.ta_lookback_slider_bounds else {
            return;
        };
        self.ta_lookback_scrubbing = true;
        self.set_ta_lookback_period(
            node_id,
            self.lookback_from_slider_position(mouse_x, bounds),
        );
        cx.notify();
    }

    pub(crate) fn update_ta_lookback_scrub(&mut self, node_id: usize, mouse_x: f32, cx: &mut Context<Self>) {
        if !self.ta_lookback_scrubbing {
            return;
        }
        let Some(bounds) = self.ta_lookback_slider_bounds else {
            return;
        };
        let next = self.lookback_from_slider_position(mouse_x, bounds);
        let current = self
            .nodes
            .iter()
            .find(|node| node.id == node_id)
            .map(|node| node.ta_lookback_period)
            .unwrap_or(DEFAULT_TA_LOOKBACK as u32);
        if next != current {
            self.set_ta_lookback_period(node_id, next);
            cx.notify();
        }
    }

    pub(crate) fn end_ta_lookback_scrub(&mut self, cx: &mut Context<Self>) {
        if !self.ta_lookback_scrubbing {
            return;
        }
        self.ta_lookback_scrubbing = false;
        self.commit_ta_parameter_change(cx);
    }

    pub(crate) fn adjust_ta_lookback_period(&mut self, node_id: usize, delta: i32, cx: &mut Context<Self>) {
        let current = self
            .nodes
            .iter()
            .find(|node| node.id == node_id)
            .map(|node| node.ta_lookback_period)
            .unwrap_or(DEFAULT_TA_LOOKBACK as u32);
        let next = clamp_ta_lookback(current.saturating_add_signed(delta) as usize) as u32;
        if next != current {
            self.set_ta_lookback_period(node_id, next);
            self.commit_ta_parameter_change(cx);
        }
    }

    pub(crate) fn set_ta_sidebar_algorithm(&mut self, node_id: usize, algorithm_id: &str, cx: &mut Context<Self>) {
        self.set_ta_indicator(node_id, algorithm_id.to_string(), cx);
    }

    pub(crate) fn selected_asset_node(&self) -> Option<&VisualNode> {
        let selected_id = self.selected_node_id?;
        self.nodes.iter().find(|node| {
            node.id == selected_id && node.node_type.is_asset_adaptor()
        })
    }

    pub(crate) fn sync_inspector_from_selection(&mut self, cx: &mut Context<Self>) {
        self.reset_otl_script_input();
        self.sync_asset_path_draft_from_selection(cx);
        self.sync_ta_inspector_category_from_selection();
    }

    pub(crate) fn sync_ta_inspector_category_from_selection(&mut self) {
        if self.selected_technical_analysis_node().is_some() {
            self.ta_inspector_category = self
                .selected_technical_analysis_node()
                .and_then(|node| node.ta_indicator_id.as_deref())
                .and_then(ta_category_for_indicator)
                .or_else(|| {
                    ta_indicator_catalog_hierarchy()
                        .first()
                        .map(|category| category.id.clone())
                });
        } else {
            self.ta_inspector_category = None;
        }
    }

    fn set_ta_inspector_category(&mut self, category_id: String, cx: &mut Context<Self>) {
        self.ta_inspector_category = Some(category_id);
        cx.notify();
    }

    pub(crate) fn sync_asset_path_draft_from_selection(&mut self, cx: &mut Context<Self>) {
        let path = self
            .selected_asset_node()
            .and_then(|node| node.asset_source.as_ref())
            .map(|source| match source {
                AssetSourceType::Csv { path } => path.clone(),
            })
            .unwrap_or_default();
        self.asset_path_input.update(cx, |input, cx| {
            input.set_content(path, cx);
        });
    }
    pub(crate) fn reload_asset_chart_from_path(&mut self, node_id: usize, path: &str) {
        match load_yahoo_finance_csv(path) {
            Ok((_, rows)) => {
                self.asset_chart_history
                    .insert(node_id, chart_buffer_from_csv_rows(&rows));
                let ohlc_bars = ohlc_bars_from_csv_rows(&rows);
                if ohlc_bars.is_empty() {
                    self.asset_ohlc_history.remove(&node_id);
                } else {
                    self.asset_ohlc_history
                        .insert(node_id, ohlc_bars.clone());
                    if let Some(node) = self.nodes.iter().find(|node| node.id == node_id) {
                        hydrate_market_stage_from_ohlc(
                            &mut self.market_stage,
                            &node.name,
                            &ohlc_bars,
                        );
                    }
                }
                self.sync_playhead_bounds();
                self.sync_playhead_time_from_index();
                self.synchronize_inspector_view();
            }
            Err(error) => {
                self.push_status_log(format!("Chart reload failed for `{path}`: {error}"));
            }
        }
    }

    pub(crate) fn apply_asset_path_to_node(
        &mut self,
        node_id: usize,
        path: String,
        cx: &mut Context<Self>,
    ) {
        if let Some(node) = self
            .nodes
            .iter_mut()
            .find(|node| node.id == node_id && node.node_type.is_asset_adaptor())
        {
            node.node_type = NodeType::asset_adaptor_from_csv_path(&path);
            node.asset_source = Some(AssetSourceType::Csv {
                path: path.clone(),
            });
            node.name = csv_node_label_from_path(&path);
            self.csv_path_registry.set_path(node_id, path.clone());
            self.reload_asset_chart_from_path(node_id, &path);
            self.sync_pipeline_graph(cx);
        }
    }

    pub(crate) fn apply_asset_path_to_selected_node(
        &mut self,
        path: String,
        cx: &mut Context<Self>,
    ) {
        let Some(selected_id) = self.selected_node_id else {
            return;
        };
        self.apply_asset_path_to_node(selected_id, path, cx);
    }

    pub(crate) fn on_asset_path_input_event(&mut self, event: &PathInputEvent, cx: &mut Context<Self>) {
        match event {
            PathInputEvent::Changed(path) => {
                self.apply_asset_path_to_selected_node(path.clone(), cx);
            }
            PathInputEvent::Submit => {
                cx.notify();
            }
        }
    }

    pub(crate) fn normalize_picked_csv_path(path: PathBuf) -> String {
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        path.strip_prefix(&manifest)
            .map(|relative| relative.to_string_lossy().replace('\\', "/"))
            .unwrap_or_else(|_| path.to_string_lossy().replace('\\', "/"))
    }

    pub(crate) fn prompt_csv_for_node(&mut self, node_id: usize, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Open".into()),
        });
        let view = cx.entity().downgrade();
        cx.spawn(async move |_, cx| {
            let picked = match receiver.await {
                Ok(Ok(Some(paths))) => paths.into_iter().next(),
                _ => None,
            };
            let _ = cx.update(|cx| {
                if let Some(view) = view.upgrade() {
                    view.update(cx, |workspace, cx| {
                        if let Some(path) = picked {
                            let path_str = Self::normalize_picked_csv_path(path);
                            workspace.selected_node_id = Some(node_id);
                            workspace.asset_path_input.update(cx, |input, cx| {
                                input.set_content(path_str.clone(), cx);
                            });
                            workspace.apply_asset_path_to_node(node_id, path_str, cx);
                            workspace.push_status_log(format!(
                                "CSV Asset bound — node {node_id} loaded"
                            ));
                        } else {
                            workspace.push_status_log(format!(
                                "CSV Asset node {node_id} created — use Browse to bind a file"
                            ));
                        }
                        cx.notify();
                    });
                }
            });
        })
        .detach();
    }

    pub(crate) fn browse_csv_asset_path(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(node) = self.selected_asset_node() else {
            return;
        };
        let node_id = node.id;
        self.prompt_csv_for_node(node_id, cx);
    }

    pub(crate) fn render_asset_path_config_row(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .mt_2()
            .mb_2()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(0xa1a1aa))
                    .child("📁 Data Stream Target Path:"),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .p_2()
                            .bg(rgb(0x141417))
                            .border_1()
                            .border_color(rgb(0x222227))
                            .rounded_sm()
                            .child(render_asset_path_input(&self.asset_path_input)),
                    )
                    .child(
                        div()
                            .px_3()
                            .py_2()
                            .bg(rgb(0x2563eb))
                            .rounded_sm()
                            .cursor(CursorStyle::PointingHand)
                            .text_size(px(10.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(0xffffff))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _event: &MouseDownEvent, window, cx| {
                                    this.browse_csv_asset_path(window, cx);
                                    cx.stop_propagation();
                                }),
                            )
                            .child("Browse…"),
                    ),
            )
    }
    pub(crate) fn render_spreadsheet_inspector(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let inspector_title = match self.selected_node_id {
            None => "📊 Global Register Inspector".to_string(),
            Some(selected_id) => {
                let node_name = self
                    .nodes
                    .iter()
                    .find(|node| node.id == selected_id)
                    .map(|node| node.name.as_str())
                    .unwrap_or("Unknown Node");
                format!("📊 Inspector Context // {node_name}")
            }
        };

        let show_asset_config = self.selected_asset_node().is_some();
        let show_ta_picker = self.selected_technical_analysis_node().is_some();
        let show_portfolio_analytics = self.selected_portfolio_node().is_some();

        // Hide live polling rows while in per-node Inspector Context; global view still shows all rows.
        let visible_rows: Vec<&MatrixDataRow> = match self.selected_node_id {
            None => self.inspector_data.iter().collect(),
            Some(_) => Vec::new(),
        };

        let mut rows = div().flex_col().gap_1().mt_3();
        if !visible_rows.is_empty() {
            for row in visible_rows {
                rows = rows.child(
                    div()
                        .flex()
                        .justify_between()
                        .p_1()
                        .bg(rgb(0x141417))
                        .border_1()
                        .border_color(rgb(0x222227))
                        .font_family("monospace")
                        .text_size(px(9.0))
                        .child(div().w_12().text_color(rgb(0x71717a)).child(row.tick.clone()))
                        .child(div().w_16().text_color(rgb(0xffffff)).child(row.asset.clone()))
                        .child(
                            div()
                                .w_20()
                                .text_color(rgb(0x38bdf8))
                                .child(row.grade_type.clone()),
                        )
                        .child(
                            div()
                                .flex_1()
                                .text_right()
                                .text_color(rgb(0x10b981))
                                .child(row.multivector_value.clone()),
                        ),
                );
            }
        }

        let mut inspector = div()
            .flex_1()
            .min_h_0()
            .overflow_hidden()
            .p_4()
            .flex_col()
            .child(
                div()
                    .flex_shrink_0()
                    .text_xs()
                    .font_weight(FontWeight::BOLD)
                    .text_color(rgb(0xe4e4e7))
                    .child(inspector_title),
            );

        if show_asset_config {
            inspector = inspector.child(
                div()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .child(self.render_asset_path_config_row(cx)),
            );
        } else if self.selected_node_id.is_none() {
            inspector = inspector.child(
                div()
                    .flex_1()
                    .min_h_0()
                    .mt_3()
                    .overflow_y_scrollbar()
                    .child(rows),
            );
        } else if show_ta_picker {
            inspector = inspector.child(
                div()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .flex_col()
                    .child(self.render_ta_parameter_controls(cx))
                    .child(self.render_ta_indicator_picker(cx)),
            );
        } else if show_portfolio_analytics {
            inspector = inspector.child(
                div()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .child(self.render_portfolio_analytics_panel()),
            );
        }

        inspector
    }

    pub(crate) fn render_portfolio_analytics_panel(&self) -> impl IntoElement {
        let Some(metrics) = &self.portfolio_diagnostics else {
            return div()
                .mt_3()
                .p_3()
                .rounded_md()
                .bg(rgb(0x141417))
                .border_1()
                .border_color(rgb(0x222227))
                .text_size(px(10.0))
                .font_family("monospace")
                .text_color(rgb(0x71717a))
                .child("Simulation ledger warming up — metrics appear after the first CSV tick.");
        };

        let tick_label = metrics
            .tick_label
            .clone()
            .unwrap_or_else(|| format_tick_label(metrics.tick_index));
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
        let wired_sources = self
            .selected_node_id
            .map(|portfolio_id| self.portfolio_wired_sources(portfolio_id))
            .unwrap_or_default();
        let activity_summary = if metrics.trade_count == 0 {
            format!(
                "{} bars · 0 trades · sat in cash (avg exposure {:.0}%)",
                metrics.bars_processed,
                metrics.avg_exposure_pct * 100.0
            )
        } else {
            format!(
                "{} bars · {} trades · avg exposure {:.0}%",
                metrics.bars_processed,
                metrics.trade_count,
                metrics.avg_exposure_pct * 100.0
            )
        };

        div()
            .mt_3()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .text_size(px(9.0))
                    .font_family("monospace")
                    .text_color(rgb(0x71717a))
                    .child(format!(
                        "Layer 2 ledger · epoch {} · {tick_label} · base {}",
                        metrics.simulation_epoch,
                        format_currency(SIM_INITIAL_CASH)
                    )),
            )
            .child(
                div()
                    .p_3()
                    .rounded_md()
                    .bg(rgb(0x111114))
                    .border_1()
                    .border_color(rgb(0x222227))
                    .text_size(px(9.0))
                    .font_family("monospace")
                    .text_color(rgb(0x94a3b8))
                    .child(activity_summary),
            )
            .when(!wired_sources.is_empty(), |panel| {
                panel.child(
                    div()
                        .p_3()
                        .rounded_md()
                        .bg(rgb(0x141417))
                        .border_1()
                        .border_color(rgb(0x222227))
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .text_size(px(9.0))
                                .font_family("monospace")
                                .text_color(rgb(0x71717a))
                                .child(format!(
                                    "Wired execution sources ({})",
                                    wired_sources.len()
                                )),
                        )
                        .children(wired_sources.into_iter().map(|(node_id, name)| {
                            div()
                                .text_size(px(10.0))
                                .font_family("monospace")
                                .text_color(rgb(0xcbd5e1))
                                .child(format!("node {node_id} · {name}"))
                        })),
                )
            })
            .child(
                div()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .p_3()
                            .rounded_md()
                            .bg(rgb(0x141417))
                            .border_1()
                            .border_color(rgb(0x222227))
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_size(px(9.0))
                                    .font_family("monospace")
                                    .text_color(rgb(0x71717a))
                                    .child("Total Return (R_total)"),
                            )
                            .child(
                                div()
                                    .text_size(px(18.0))
                                    .font_weight(FontWeight::BOLD)
                                    .font_family("monospace")
                                    .text_color(return_color)
                                    .child(format_percent_signed(metrics.total_return_pct)),
                            )
                            .child(
                                div()
                                    .text_size(px(9.0))
                                    .font_family("monospace")
                                    .text_color(rgb(0x52525b))
                                    .child(format!(
                                        "NAV {} vs base {}",
                                        format_currency(metrics.nav),
                                        format_currency(SIM_INITIAL_CASH)
                                    )),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(
                                div()
                                    .flex_1()
                                    .p_3()
                                    .rounded_md()
                                    .bg(rgb(0x141417))
                                    .border_1()
                                    .border_color(rgb(0x222227))
                                    .flex_col()
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_size(px(9.0))
                                            .font_family("monospace")
                                            .text_color(rgb(0x71717a))
                                            .child("Buy & Hold Benchmark"),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(16.0))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .font_family("monospace")
                                            .text_color(rgb(0xe2e8f0))
                                            .child(
                                                metrics
                                                    .benchmark_return_pct
                                                    .map(format_percent_signed)
                                                    .unwrap_or_else(|| "—".to_string()),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(9.0))
                                            .font_family("monospace")
                                            .text_color(rgb(0x52525b))
                                            .child("Same-window asset return"),
                                    ),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .p_3()
                                    .rounded_md()
                                    .bg(rgb(0x141417))
                                    .border_1()
                                    .border_color(rgb(0x222227))
                                    .flex_col()
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_size(px(9.0))
                                            .font_family("monospace")
                                            .text_color(rgb(0x71717a))
                                            .child("Alpha (vs B&H)"),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(16.0))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .font_family("monospace")
                                            .text_color(alpha_color)
                                            .child(
                                                metrics
                                                    .excess_return_pct
                                                    .map(format_percent_signed)
                                                    .unwrap_or_else(|| "—".to_string()),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(9.0))
                                            .font_family("monospace")
                                            .text_color(rgb(0x52525b))
                                            .child("Strategy return minus benchmark"),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(
                                div()
                                    .flex_1()
                                    .p_3()
                                    .rounded_md()
                                    .bg(rgb(0x141417))
                                    .border_1()
                                    .border_color(rgb(0x222227))
                                    .flex_col()
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_size(px(9.0))
                                            .font_family("monospace")
                                            .text_color(rgb(0x71717a))
                                            .child("Max Drawdown (MDD)"),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(16.0))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .font_family("monospace")
                                            .text_color(rgb(0xf59e0b))
                                            .child(format_percent_magnitude(
                                                metrics.max_drawdown_pct,
                                            )),
                                    ),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .p_3()
                                    .rounded_md()
                                    .bg(rgb(0x141417))
                                    .border_1()
                                    .border_color(rgb(0x222227))
                                    .flex_col()
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_size(px(9.0))
                                            .font_family("monospace")
                                            .text_color(rgb(0x71717a))
                                            .child("Sharpe Ratio (S)"),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(16.0))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .font_family("monospace")
                                            .text_color(rgb(0x38bdf8))
                                            .child(format_ratio(metrics.sharpe_ratio)),
                                    ),
                            ),
                    ),
            )
            .child(
                div()
                    .p_3()
                    .rounded_md()
                    .bg(rgb(0x111114))
                    .border_1()
                    .border_color(rgb(0x222227))
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_size(px(9.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .font_family("monospace")
                            .text_color(rgb(0x94a3b8))
                            .child("Position Ledger"),
                    )
                    .child(
                        div()
                            .flex()
                            .justify_between()
                            .text_size(px(9.0))
                            .font_family("monospace")
                            .text_color(rgb(0x71717a))
                            .child("Cash")
                            .child(
                                div()
                                    .text_color(rgb(0xe4e4e7))
                                    .child(format_currency(metrics.cash)),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .justify_between()
                            .text_size(px(9.0))
                            .font_family("monospace")
                            .text_color(rgb(0x71717a))
                            .child("Active Positions")
                            .child(
                                div()
                                    .text_color(rgb(0xe4e4e7))
                                    .child(format!("{:.4}", metrics.position_qty)),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .justify_between()
                            .text_size(px(9.0))
                            .font_family("monospace")
                            .text_color(rgb(0x71717a))
                            .child("Mark Price")
                            .child(
                                div()
                                    .text_color(rgb(0xe4e4e7))
                                    .child(format_currency(metrics.mark_price)),
                            ),
                    ),
            )
    }

    pub(crate) fn render_ta_parameter_controls(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(node) = self.selected_technical_analysis_node() else {
            return div().into_any_element();
        };
        let node_id = node.id;
        let lookback = node.ta_lookback_period;
        let active_algorithm = node
            .ta_indicator_id
            .as_deref()
            .unwrap_or(DEFAULT_TA_INDICATOR_ID);
        let lookback_span = (MAX_TA_LOOKBACK - MIN_TA_LOOKBACK) as f32;
        let slider_fraction = if lookback_span <= f32::EPSILON {
            0.0
        } else {
            (lookback as f32 - MIN_TA_LOOKBACK as f32) / lookback_span
        };
        let view = cx.entity().downgrade();

        let mut algorithm_row = div().flex().flex_row().gap_1().mt_2();
        for (algorithm_id, label) in TA_SIDEBAR_ALGORITHMS {
            let is_active = active_algorithm.eq_ignore_ascii_case(algorithm_id);
            algorithm_row = algorithm_row.child(
                div()
                    .px_2()
                    .py_1()
                    .rounded_sm()
                    .cursor_pointer()
                    .bg(if is_active {
                        rgb(0x2a1f3d)
                    } else {
                        rgb(0x141417)
                    })
                    .border_1()
                    .border_color(if is_active {
                        rgb(0xa855f7)
                    } else {
                        rgb(0x222227)
                    })
                    .text_size(px(9.0))
                    .font_weight(if is_active {
                        FontWeight::SEMIBOLD
                    } else {
                        FontWeight::NORMAL
                    })
                    .text_color(if is_active {
                        rgb(0xe9d5ff)
                    } else {
                        rgb(0xa1a1aa)
                    })
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _event: &MouseDownEvent, _window, cx| {
                            this.set_ta_sidebar_algorithm(node_id, algorithm_id, cx);
                            cx.stop_propagation();
                        }),
                    )
                    .child(*label),
            );
        }

        div()
            .flex_shrink_0()
            .flex_col()
            .gap_2()
            .p_3()
            .bg(rgb(0x111114))
            .border_1()
            .border_color(rgb(0x222227))
            .rounded_md()
            .child(
                div()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(0xc084fc))
                    .child("TA Parameters"),
            )
            .child(algorithm_row)
            .child(
                div()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .flex()
                            .justify_between()
                            .items_center()
                            .child(
                                div()
                                    .text_size(px(9.0))
                                    .text_color(rgb(0xa1a1aa))
                                    .child("Lookback Period"),
                            )
                            .child(
                                div()
                                    .text_size(px(9.0))
                                    .font_family("monospace")
                                    .text_color(rgb(0xe9d5ff))
                                    .child(format!("{lookback} bars")),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .px_2()
                                    .py_1()
                                    .rounded_sm()
                                    .cursor_pointer()
                                    .bg(rgb(0x141417))
                                    .border_1()
                                    .border_color(rgb(0x222227))
                                    .text_size(px(10.0))
                                    .font_family("monospace")
                                    .text_color(rgb(0xe9d5ff))
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(move |this, _event: &MouseDownEvent, _window, cx| {
                                            this.adjust_ta_lookback_period(node_id, -1, cx);
                                            cx.stop_propagation();
                                        }),
                                    )
                                    .child("−"),
                            )
                            .child(
                                div()
                                    .text_size(px(10.0))
                                    .font_family("monospace")
                                    .text_color(rgb(0xe9d5ff))
                                    .child(format!("{lookback} bars")),
                            )
                            .child(
                                div()
                                    .px_2()
                                    .py_1()
                                    .rounded_sm()
                                    .cursor_pointer()
                                    .bg(rgb(0x141417))
                                    .border_1()
                                    .border_color(rgb(0x222227))
                                    .text_size(px(10.0))
                                    .font_family("monospace")
                                    .text_color(rgb(0xe9d5ff))
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(move |this, _event: &MouseDownEvent, _window, cx| {
                                            this.adjust_ta_lookback_period(node_id, 1, cx);
                                            cx.stop_propagation();
                                        }),
                                    )
                                    .child("+"),
                            ),
                    )
                    .child(
                        div()
                            .relative()
                            .h(px(16.0))
                            .rounded_sm()
                            .bg(rgb(0x18181b))
                            .border_1()
                            .border_color(rgb(0x27272a))
                            .cursor(CursorStyle::PointingHand)
                            .on_children_prepainted({
                                move |bounds: Vec<Bounds<Pixels>>, _window: &mut Window, cx: &mut App| {
                                    if let Some(track_bounds) = bounds.last() {
                                        let _ = view.update(cx, |workspace, _cx| {
                                            workspace.ta_lookback_slider_bounds = Some(*track_bounds);
                                        });
                                    }
                                }
                            })
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(
                                    move |this, event: &MouseDownEvent, _window, cx| {
                                        this.begin_ta_lookback_scrub(node_id, event.position.x.into(), cx);
                                        cx.stop_propagation();
                                    },
                                ),
                            )
                            .on_mouse_move(cx.listener(
                                move |this, event: &MouseMoveEvent, _window, cx| {
                                    this.update_ta_lookback_scrub(node_id, event.position.x.into(), cx);
                                },
                            ))
                            .on_mouse_up(
                                MouseButton::Left,
                                cx.listener(move |this, _event: &MouseUpEvent, _window, cx| {
                                    this.end_ta_lookback_scrub(cx);
                                    cx.stop_propagation();
                                }),
                            )
                            .on_mouse_up_out(
                                MouseButton::Left,
                                cx.listener(move |this, _event: &MouseUpEvent, _window, cx| {
                                    this.end_ta_lookback_scrub(cx);
                                }),
                            )
                            .child(
                                div()
                                    .absolute()
                                    .top_0()
                                    .left_0()
                                    .h_full()
                                    .w(relative(slider_fraction.max(0.01)))
                                    .bg(rgb(0x581c87)),
                            )
                            .child(
                                div()
                                    .absolute()
                                    .top(px(-2.0))
                                    .left(relative(slider_fraction.clamp(0.0, 1.0)))
                                    .w(px(8.0))
                                    .h(px(16.0))
                                    .rounded_full()
                                    .bg(rgb(0xa855f7)),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .justify_between()
                            .text_size(px(8.0))
                            .font_family("monospace")
                            .text_color(rgb(0x71717a))
                            .child(format!("{MIN_TA_LOOKBACK}"))
                            .child(format!("{MAX_TA_LOOKBACK}")),
                    ),
            )
            .into_any_element()
    }

    pub(crate) fn render_ta_indicator_picker(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let selected_id = self.selected_node_id;
        let selected_indicator = self
            .selected_technical_analysis_node()
            .and_then(|node| node.ta_indicator_id.clone());
        let hierarchy = ta_indicator_catalog_hierarchy();
        let catalog_len: usize = hierarchy.iter().map(|category| category.count()).sum();

        let active_category = self
            .ta_inspector_category
            .clone()
            .filter(|category_id| hierarchy.iter().any(|category| category.id == *category_id))
            .or_else(|| hierarchy.first().map(|category| category.id.clone()))
            .unwrap_or_default();

        let active_category_label = hierarchy
            .iter()
            .find(|category| category.id == active_category)
            .map(|category| category.label.clone())
            .unwrap_or_else(|| category_display_label(&active_category));

        let current_binding = selected_indicator.as_deref().and_then(|indicator_id| {
            let label = ta_indicator_label(indicator_id).unwrap_or(indicator_id);
            let category = ta_category_for_indicator(indicator_id)
                .map(|id| category_display_label(&id))
                .unwrap_or_default();
            Some(format!("{label} · {category}"))
        });

        let mut shelf = div()
            .w(px(108.0))
            .flex_shrink_0()
            .min_h_0()
            .flex_col()
            .gap_0p5()
            .py_1()
            .bg(rgb(0x111114))
            .border_r_1()
            .border_color(rgb(0x222227))
            .overflow_y_scrollbar();

        for category in &hierarchy {
            let category_id = category.id.clone();
            let is_active = category_id == active_category;
            let shelf_bg = if is_active {
                rgb(0x2a1f3d)
            } else {
                rgb(0x111114)
            };
            let accent = if is_active {
                rgb(0xa855f7)
            } else {
                rgb(0x2d2d34)
            };

            shelf = shelf.child(
                div()
                    .px_1p5()
                    .py_1()
                    .bg(shelf_bg)
                    .border_l_2()
                    .border_color(accent)
                    .cursor_pointer()
                    .hover(|style| style.bg(rgb(0x25252b)))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _event: &MouseDownEvent, _window, cx| {
                            this.set_ta_inspector_category(category_id.clone(), cx);
                            cx.stop_propagation();
                        }),
                    )
                    .child(
                        div()
                            .text_size(px(8.0))
                            .font_weight(if is_active {
                                FontWeight::SEMIBOLD
                            } else {
                                FontWeight::NORMAL
                            })
                            .text_color(if is_active {
                                rgb(0xe9d5ff)
                            } else {
                                rgb(0xa1a1aa)
                            })
                            .child(category.label.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(7.0))
                            .font_family("monospace")
                            .text_color(rgb(0x71717a))
                            .child(format!("{}", category.count())),
                    ),
            );
        }

        let active_entries = hierarchy
            .iter()
            .find(|category| category.id == active_category)
            .map(|category| category.entries.as_slice())
            .unwrap_or(&[]);

        let mut list = div()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .flex_col()
            .gap_0p5()
            .p_1()
            .overflow_y_scrollbar();

        for entry in active_entries {
            let indicator_id = entry.id.clone();
            let is_selected = selected_indicator.as_deref() == Some(indicator_id.as_str());
            let row_bg = if is_selected {
                rgb(0x2a1f3d)
            } else {
                rgb(0x141417)
            };
            list = list.child(
                div()
                    .p_1p5()
                    .bg(row_bg)
                    .border_1()
                    .border_color(if is_selected {
                        rgb(0xa855f7)
                    } else {
                        rgb(0x222227)
                    })
                    .rounded_sm()
                    .cursor_pointer()
                    .hover(|style| style.bg(rgb(0x25252b)))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _event: &MouseDownEvent, _window, cx| {
                            if let Some(node_id) = selected_id {
                                this.set_ta_indicator(node_id, indicator_id.clone(), cx);
                            }
                            cx.stop_propagation();
                        }),
                    )
                    .child(
                        div()
                            .text_size(px(9.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(0xe9d5ff))
                            .child(entry.label.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(8.0))
                            .font_family("monospace")
                            .text_color(rgb(0x71717a))
                            .child(entry.id.clone()),
                    ),
            );
        }

        div()
            .flex_1()
            .min_h_0()
            .flex_col()
            .gap_2()
            .mt_2()
            .child(
                div()
                    .flex_shrink_0()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(0xc084fc))
                            .child(format!("VectorTA ({catalog_len})")),
                    )
                    .child(
                        div()
                            .text_size(px(8.0))
                            .text_color(rgb(0x71717a))
                            .child(format!(
                                "Shelf: {active_category_label} · {} indicators",
                                active_entries.len()
                            )),
                    )
                    .when(current_binding.is_some(), |header| {
                        header.child(
                            div()
                                .text_size(px(8.0))
                                .font_family("monospace")
                                .text_color(rgb(0x38bdf8))
                                .child(format!(
                                    "Bound: {}",
                                    current_binding.clone().unwrap_or_default()
                                )),
                        )
                    }),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .flex()
                    .flex_row()
                    .border_1()
                    .border_color(rgb(0x222227))
                    .rounded_md()
                    .overflow_hidden()
                    .child(shelf)
                    .child(list),
            )
    }
}
