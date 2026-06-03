//! Spreadsheet inspector sidebar: parameters, ledgers, and analytics panels.

use gpui::*;
use gpui::prelude::FluentBuilder;
use gpui_component::scroll::ScrollableElement;

use crate::asset_path_input::render_asset_path_input;
use crate::graph_compiler::{AssetSourceType, NodeType, VisualNode};
use pulsar_marketlab::technical_analysis::{
    clamp_ta_lookback, DEFAULT_TA_LOOKBACK, MAX_TA_LOOKBACK, MIN_TA_LOOKBACK,
};
use pulsar_marketlab_core::{algorithm_display_label, hyperparameter_visibility, TaArchetype};

use crate::ui::ta_uber_inspector::{
    adjust_period, adjust_signal_period, algorithm_picker_chip, archetype_summary,
    hyperparam_stepper, ta_header_tint,
};
use std::path::PathBuf;
use crate::asset_path_input::PathInputEvent;
use crate::workspace_state::{
    chart_buffer_from_csv_rows, csv_node_label_from_path, hydrate_market_stage_from_ohlc,
    load_yahoo_finance_csv, ohlc_bars_from_csv_rows, format_currency, format_percent_magnitude,
    format_percent_signed, format_ratio, format_tick_label, MatrixDataRow, TradingSystemWorkspace,
    SIM_INITIAL_CASH,
};
use crate::portfolio_integrator_ledger::{
    render_integrator_ledger_spreadsheet, IntegratorLedgerFilter, IntegratorLedgerHost,
};
use crate::portfolio_wealth_chart::{
    build_portfolio_wealth_chart_from_streams, render_portfolio_wealth_chart,
    PortfolioChartHost, PortfolioChartOverlayKey,
};

impl TradingSystemWorkspace {
    pub(crate) fn selected_technical_analysis_node(&self) -> Option<&VisualNode> {
        let selected_id = self.selected_node_id?;
        self.nodes.iter().find(|node| {
            node.id == selected_id && node.node_type.is_ta_uber_signal()
        })
    }

    pub(crate) fn selected_portfolio_node(&self) -> Option<&VisualNode> {
        let selected_id = self.selected_node_id?;
        self.nodes.iter().find(|node| {
            node.id == selected_id && node.node_type.is_portfolio()
        })
    }

    pub(crate) fn commit_ta_parameter_change(&mut self, cx: &mut Context<Self>) {
        self.commit_ta_uber_parameter_change(cx);
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
        self.set_ta_period_for_node(
            node_id,
            self.lookback_from_slider_position(mouse_x, bounds),
            cx,
        );
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
            .map(|node| node.overlay_period().unwrap_or(14))
            .unwrap_or(DEFAULT_TA_LOOKBACK as u32);
        if next != current {
            self.set_ta_period_for_node(node_id, next, cx);
        }
    }

    pub(crate) fn end_ta_lookback_scrub(&mut self, _cx: &mut Context<Self>) {
        self.ta_lookback_scrubbing = false;
    }

    pub(crate) fn adjust_ta_lookback_period(&mut self, node_id: usize, delta: i32, cx: &mut Context<Self>) {
        crate::ui::ta_uber_inspector::adjust_period(self, node_id, delta, cx);
    }

    pub(crate) fn selected_asset_node(&self) -> Option<&VisualNode> {
        let selected_id = self.selected_node_id?;
        self.nodes.iter().find(|node| {
            node.id == selected_id && node.node_type.is_asset_adaptor()
        })
    }

    pub(crate) fn sync_inspector_from_selection(&mut self, cx: &mut Context<Self>) {
        self.reset_otl_script_input();
        self.reset_otl_editor_input();
        self.sync_asset_path_draft_from_selection(cx);
        self.sync_ta_inspector_category_from_selection();
    }

    pub(crate) fn sync_ta_inspector_category_from_selection(&mut self) {
        self.ta_inspector_category = self
            .selected_technical_analysis_node()
            .and_then(|node| node.node_type.ta_uber_config())
            .map(|config| config.archetype.as_token().to_string());
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
    pub(crate) fn reload_asset_chart_from_path(
        &mut self,
        node_id: usize,
        path: &str,
        cx: &mut Context<Self>,
    ) {
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
                self.snap_playhead_to_last_bar();
                self.synchronize_inspector_view();
                self.sync_view_window(cx);
                self.request_graph_engine_timeline_refresh(cx);
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
            self.reload_asset_chart_from_path(node_id, &path, cx);
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
                    .overflow_y_scrollbar()
                    .child(self.render_ta_uber_inspector(cx)),
            );
        } else if show_portfolio_analytics {
            inspector = inspector.child(
                div()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .child(self.render_portfolio_analytics_panel(cx)),
            );
        }

        inspector
    }

    pub(crate) fn render_portfolio_analytics_panel(
        &mut self,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let bridge_metrics = cx
            .global::<crate::ui::telemetry_bridge::MetricsTelemetryBridge>()
            .global_metrics
            .clone();
        let metrics = self.portfolio_diagnostics_for_selection();
        let graph_status = self.portfolio_graph_engine_status_label(cx);
        let wired_sources = self
            .selected_node_id
            .map(|portfolio_id| self.portfolio_wired_sources(portfolio_id))
            .unwrap_or_default();

        let mut panel = div()
            .mt_3()
            .flex_col()
            .gap_3()
            .child(self.render_portfolio_diagnostics_summary_section(metrics))
            .child(
                div()
                    .text_size(px(9.0))
                    .font_family("monospace")
                    .text_color(rgb(0x71717a))
                    .child(graph_status),
            )
            .when(self.graph_engine_analytics_active(), |panel| {
                panel.child(
                    div()
                        .p_3()
                        .rounded_md()
                        .bg(rgb(0x101014))
                        .border_1()
                        .border_color(rgb(0x222227))
                        .text_size(px(10.0))
                        .font_family("monospace")
                        .text_color(rgb(0xcbd5e1))
                        .child(format!(
                            "Live GraphEngine · R {} · MDD {} · Exp {:.0}% · {} trades · conv {:.2}",
                            format_percent_signed(bridge_metrics.total_return),
                            format_percent_signed(-bridge_metrics.rolling_drawdown),
                            bridge_metrics.net_exposure * 100.0,
                            bridge_metrics.trailing_trades_count,
                            bridge_metrics.current_conviction,
                        )),
                )
            })
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
                    .p_3()
                    .rounded_md()
                    .bg(rgb(0x141417))
                    .border_1()
                    .border_color(rgb(0x222227))
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .text_size(px(9.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .font_family("monospace")
                            .text_color(rgb(0x94a3b8))
                            .child("Wealth Timeline"),
                    )
                    .child(self.render_portfolio_wealth_chart_section(cx)),
            );

        if self.portfolio_integrator_ledger_for_selection().is_some() {
            panel = panel.child(
                div()
                    .p_3()
                    .rounded_md()
                    .bg(rgb(0x141417))
                    .border_1()
                    .border_color(rgb(0x222227))
                    .child({
                        let ledger = self
                            .portfolio_integrator_ledger_for_selection()
                            .expect("ledger present");
                        render_integrator_ledger_spreadsheet(
                            ledger,
                            self.portfolio_ledger_filter.clone(),
                            cx.entity(),
                        )
                    }),
            );
        }

        panel
    }

    fn render_portfolio_diagnostics_summary_section(
        &self,
        metrics: Option<&crate::workspace_state::PortfolioDiagnosticsSnapshot>,
    ) -> impl IntoElement {
        let Some(metrics) = metrics else {
            return div()
                .p_3()
                .rounded_md()
                .bg(rgb(0x111114))
                .border_1()
                .border_color(rgb(0x222227))
                .text_size(px(10.0))
                .font_family("monospace")
                .text_color(rgb(0x71717a))
                .child(
                    "Portfolio metrics appear after the first asset tick. \
                     Graph-engine wealth and integrator rows below use the compiled portfolio sweep.",
                );
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
            .flex_col()
            .gap_2()
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
            .child(self.render_portfolio_metrics_cards(metrics, return_color, alpha_color))
            .child(self.render_portfolio_position_ledger(metrics))
            .child(
                div()
                    .text_size(px(9.0))
                    .font_family("monospace")
                    .text_color(rgb(0x71717a))
                    .child(format!(
                        "CSV simulation ledger · epoch {} · {tick_label} · base {}",
                        metrics.simulation_epoch,
                        format_currency(SIM_INITIAL_CASH)
                    )),
            )
    }

    fn render_portfolio_wealth_chart_section(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        if let Some(series) = self.portfolio_wealth_chart_for_selection() {
            return render_portfolio_wealth_chart(
                series,
                self.portfolio_chart_overlays,
                self.playhead_current,
                cx.entity(),
            );
        }
        if let Some(node) = self.selected_portfolio_node() {
            let prim_path = self
                .stage_prim_path_for_node_in_graph(node)
                .unwrap_or_default();
            let streams = self.workspace_context.read(cx).computed_streams();
            if let Some(series) = build_portfolio_wealth_chart_from_streams(
                streams,
                &prim_path,
                self.timeline_bar_labels(),
            ) {
                return render_portfolio_wealth_chart(
                    &series,
                    self.portfolio_chart_overlays,
                    self.playhead_current,
                    cx.entity(),
                );
            }
            return div()
                .text_size(px(10.0))
                .font_family("monospace")
                .text_color(rgb(0x71717a))
                .child(format!(
                    "Awaiting graph-engine sweep for `{prim_path}`. \
                     Wire asset → signal → portfolio and load CSV data."
                ))
                .into_any_element();
        }
        div()
            .text_size(px(10.0))
            .font_family("monospace")
            .text_color(rgb(0x71717a))
            .child("Select a portfolio integrator node.")
            .into_any_element()
    }

    fn render_portfolio_metrics_cards(
        &self,
        metrics: &crate::workspace_state::PortfolioDiagnosticsSnapshot,
        return_color: gpui::Rgba,
        alpha_color: gpui::Rgba,
    ) -> impl IntoElement {
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
                                    .child(format_percent_magnitude(metrics.max_drawdown_pct)),
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
            )
    }

    fn render_portfolio_position_ledger(
        &self,
        metrics: &crate::workspace_state::PortfolioDiagnosticsSnapshot,
    ) -> impl IntoElement {
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
            )
    }

    pub(crate) fn render_ta_uber_inspector(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(node) = self.selected_technical_analysis_node() else {
            return div().into_any_element();
        };
        let node_id = node.id;
        let config = node
            .node_type
            .ta_uber_config()
            .expect("ta node has config")
            .clone();
        let accent = config.archetype.accent_rgb();
        let active_algorithm = config.algorithm.as_str();
        let visibility = hyperparameter_visibility(&config);

        let mut algorithm_row = div().flex().flex_row().flex_wrap().gap_1().mt_2();
        for algorithm_id in config.archetype.algorithms() {
            let label = algorithm_display_label(algorithm_id);
            let is_active = active_algorithm.eq_ignore_ascii_case(algorithm_id);
            algorithm_row = algorithm_row.child(algorithm_picker_chip(
                node_id,
                algorithm_id,
                label,
                is_active,
                accent,
                cx,
            ));
        }

        let mut panel = div()
            .flex_shrink_0()
            .flex_col()
            .gap_3()
            .p_3()
            .bg(rgb(ta_header_tint(config.archetype)))
            .border_1()
            .border_color(rgb(accent))
            .rounded_md()
            .child(
                div()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(accent))
                    .child(config.archetype.display_name()),
            )
            .child(
                div()
                    .text_size(px(9.0))
                    .text_color(rgb(0xa1a1aa))
                    .child(archetype_summary(&config)),
            )
            .child(
                div()
                    .text_size(px(9.0))
                    .text_color(rgb(0x71717a))
                    .child("Ports are fixed for this archetype; adjust hyperparameters below."),
            )
            .child(
                div()
                    .text_size(px(9.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(0xd4d4d8))
                    .child("Algorithm"),
            )
            .child(algorithm_row);

        if visibility.period {
            panel = panel.child(hyperparam_stepper(
                "Period",
                format!("{} bars", config.period),
                node_id,
                1,
                adjust_period,
                cx,
            ));
        }
        if visibility.signal_period {
            panel = panel.child(hyperparam_stepper(
                "Signal period",
                format!("{} bars", config.signal_period),
                node_id,
                1,
                adjust_signal_period,
                cx,
            ));
        }
        if visibility.multiplier {
            panel = panel.child(hyperparam_stepper(
                "Multiplier",
                format!("{:.2}", config.multiplier),
                node_id,
                1,
                |this, id, delta, cx| {
                    let current = this
                        .nodes
                        .iter()
                        .find(|n| n.id == id)
                        .and_then(|n| n.node_type.ta_uber_config())
                        .map(|c| c.multiplier)
                        .unwrap_or(2.0);
                    let next = (current + delta as f32 * 0.25).max(0.25);
                    this.set_ta_multiplier_for_node(id, next, cx);
                },
                cx,
            ));
        }
        if visibility.annualization {
            panel = panel.child(hyperparam_stepper(
                "Annualization",
                format!("{:.0}", config.annualization),
                node_id,
                1,
                |this, id, delta, cx| {
                    let current = this
                        .nodes
                        .iter()
                        .find(|n| n.id == id)
                        .and_then(|n| n.node_type.ta_uber_config())
                        .map(|c| c.annualization)
                        .unwrap_or(252.0);
                    let next = (current + delta as f32 * 21.0).max(1.0);
                    this.set_ta_annualization_for_node(id, next, cx);
                },
                cx,
            ));
        }

        panel.into_any_element()
    }

    #[allow(dead_code)]
    pub(crate) fn render_ta_parameter_controls(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        self.render_ta_uber_inspector(cx)
    }

    #[allow(dead_code)]
    pub(crate) fn render_ta_indicator_picker(&mut self, _cx: &mut Context<Self>) -> impl IntoElement {
        div().into_any_element()
    }

    pub(crate) fn set_portfolio_chart_overlay_toggle(
        &mut self,
        overlay: PortfolioChartOverlayKey,
        enabled: bool,
        cx: &mut Context<Self>,
    ) {
        match overlay {
            PortfolioChartOverlayKey::DrawdownShading => {
                self.portfolio_chart_overlays.drawdown_shading = enabled;
            }
            PortfolioChartOverlayKey::SignalMarkers => {
                self.portfolio_chart_overlays.signal_markers = enabled;
            }
            PortfolioChartOverlayKey::RegimeScaleShifts => {
                self.portfolio_chart_overlays.regime_scale_shifts = enabled;
            }
        }
        cx.notify();
    }
}

impl PortfolioChartHost for TradingSystemWorkspace {
    fn set_portfolio_chart_overlay(
        &mut self,
        overlay: PortfolioChartOverlayKey,
        enabled: bool,
        cx: &mut Context<Self>,
    ) {
        self.set_portfolio_chart_overlay_toggle(overlay, enabled, cx);
    }
}

impl IntegratorLedgerHost for TradingSystemWorkspace {
    fn set_integrator_ledger_filter(
        &mut self,
        filter: IntegratorLedgerFilter,
        cx: &mut Context<Self>,
    ) {
        self.portfolio_ledger_filter = filter;
        cx.notify();
    }

    fn export_integrator_ledger_csv(&mut self, cx: &mut Context<Self>) {
        let Some(ledger) = self.portfolio_integrator_ledger_for_selection() else {
            self.push_status_log("Integrator ledger export skipped — no cached matrix.".to_string());
            return;
        };
        let rows: Vec<_> = ledger
            .rows_for_filter(&self.portfolio_ledger_filter)
            .into_iter()
            .cloned()
            .collect();
        if rows.is_empty() {
            self.push_status_log("Integrator ledger export skipped — filter returned 0 rows.".to_string());
            return;
        }

        let csv = crate::portfolio_integrator_ledger::ledger_csv_content(&rows);
        let default_name = self
            .selected_portfolio_node()
            .map(|node| format!("{}_integrator_ledger.csv", node.name.replace(' ', "_")))
            .unwrap_or_else(|| "portfolio_integrator_ledger.csv".to_string());

        cx.spawn(async move |this, cx| {
            let picked = cx
                .background_executor()
                .spawn(async move {
                    rfd::AsyncFileDialog::new()
                        .set_file_name(default_name)
                        .add_filter("CSV", &["csv"])
                        .save_file()
                        .await
                        .map(|handle| handle.path().to_path_buf())
                })
                .await;

            let Some(path) = picked else {
                return;
            };

            let write_result = std::fs::write(&path, csv.as_bytes());
            let _ = cx.update(|cx| {
                if let Some(entity) = this.upgrade() {
                    entity.update(cx, |workspace, cx| {
                        match write_result {
                            Ok(()) => workspace.push_status_log(format!(
                                "Integrator ledger exported — {} rows to `{}`",
                                rows.len(),
                                path.display()
                            )),
                            Err(error) => workspace.push_status_log(format!(
                                "Integrator ledger export failed for `{}`: {error}",
                                path.display()
                            )),
                        }
                        cx.notify();
                    });
                }
            });
        })
        .detach();
    }
}
