//! Virtual-scroll integrator ledger spreadsheet for portfolio tracking matrix rows.

use std::sync::Arc;

use gpui::*;
use gpui::prelude::FluentBuilder;
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::scroll::ScrollableElement;
use gpui_component::Selectable;
use pulsar_marketlab_core::{PortfolioIntegrationResult, PortfolioTrackingFrame};

use crate::workspace_state::{format_percent_signed, format_tick_label};

const ROW_HEIGHT: f32 = 22.0;
const HEADER_BG: u32 = 0x1c1c21;
const ROW_A: u32 = 0x141417;
const ROW_B: u32 = 0x111114;
const ROW_RISK: u32 = 0x1a1520;
const ROW_PORTFOLIO: u32 = 0x102018;
const BORDER: u32 = 0x222227;
const TEXT: u32 = 0xcbd5e1;
const TEXT_MUTED: u32 = 0x71717a;
const TEXT_ACCENT: u32 = 0x38bdf8;
const TEXT_PORTFOLIO: u32 = 0x34d399;

/// Asset column label for aggregate portfolio NAV / return rows.
pub const PORTFOLIO_LEDGER_ASSET: &str = "PORTFOLIO";

const COL_DATE: Pixels = px(92.0);
const COL_ASSET: Pixels = px(56.0);
const COL_RAW: Pixels = px(76.0);
const COL_ALTERED: Pixels = px(76.0);
const COL_PRICE: Pixels = px(84.0);
const COL_UNITS: Pixels = px(76.0);
const COL_RETURN: Pixels = px(72.0);

/// Quick-filter mode for the integrator ledger spreadsheet.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum IntegratorLedgerFilter {
    #[default]
    AllAssets,
    RiskModifiedOnly,
    PortfolioNav,
    Asset(String),
}

impl IntegratorLedgerFilter {
    pub fn label(&self) -> String {
        match self {
            Self::AllAssets => "All assets".to_string(),
            Self::RiskModifiedOnly => "Risk-modified rows".to_string(),
            Self::PortfolioNav => "Portfolio NAV".to_string(),
            Self::Asset(symbol) => symbol.clone(),
        }
    }
}

/// One display row in the integrator historical log.
#[derive(Clone, Debug, PartialEq)]
pub struct IntegratorLedgerRow {
    pub timestamp_label: String,
    pub asset_id: String,
    pub closure_raw_weight: f64,
    pub altered_portfolio_weight: f64,
    pub nominal_price: f64,
    pub calculated_units: f64,
    pub investment_return: f64,
    pub risk_modified: bool,
    /// Aggregate NAV / bar return row (not an underlying asset leg).
    pub is_portfolio_summary: bool,
}

/// Cached ledger snapshot keyed by portfolio USD prim path.
#[derive(Clone, Debug, Default)]
pub struct PortfolioIntegratorLedger {
    pub rows: Vec<IntegratorLedgerRow>,
    pub assets: Vec<String>,
}

impl PortfolioIntegratorLedger {
    pub fn filtered_indices(&self, filter: &IntegratorLedgerFilter) -> Vec<usize> {
        self.rows
            .iter()
            .enumerate()
            .filter_map(|(index, row)| match filter {
                IntegratorLedgerFilter::AllAssets => Some(index),
                IntegratorLedgerFilter::RiskModifiedOnly if row.risk_modified && !row.is_portfolio_summary => {
                    Some(index)
                }
                IntegratorLedgerFilter::PortfolioNav if row.is_portfolio_summary => Some(index),
                IntegratorLedgerFilter::Asset(symbol)
                    if !row.is_portfolio_summary && row.asset_id == *symbol =>
                {
                    Some(index)
                }
                _ => None,
            })
            .collect()
    }

    pub fn rows_for_filter(&self, filter: &IntegratorLedgerFilter) -> Vec<&IntegratorLedgerRow> {
        self.filtered_indices(filter)
            .into_iter()
            .map(|index| &self.rows[index])
            .collect()
    }
}

pub fn build_integrator_ledger(
    integration: &PortfolioIntegrationResult,
    bar_labels: Option<Vec<String>>,
) -> PortfolioIntegratorLedger {
    let mut assets = Vec::new();
    let mut rows = Vec::new();
    let bar_count = integration
        .wealth_series
        .len()
        .max(
            integration
                .tracking_matrix
                .iter()
                .map(|frame| frame.timestamp as usize + 1)
                .max()
                .unwrap_or(0),
        );

    for bar in 0..bar_count {
        let timestamp = bar as i64;
        let leg_frames: Vec<_> = integration
            .tracking_matrix
            .iter()
            .filter(|frame| frame.timestamp == timestamp)
            .collect();

        for frame in &leg_frames {
            if !assets.iter().any(|asset| asset == &frame.asset_id) {
                assets.push(frame.asset_id.clone());
            }
            rows.push(integrator_row_from_frame(frame, &bar_labels));
        }

        if let Some(&nav) = integration.wealth_series.get(bar) {
            let prior_nav = integration
                .wealth_series
                .get(bar.saturating_sub(1))
                .copied()
                .unwrap_or(nav);
            let bar_return = if bar > 0 && prior_nav.abs() > f64::EPSILON {
                (nav - prior_nav) / prior_nav
            } else {
                0.0
            };
            let gross_exposure: f64 = leg_frames
                .iter()
                .map(|frame| frame.altered_portfolio_weight.abs())
                .sum();
            rows.push(integrator_portfolio_row(
                bar,
                nav,
                bar_return,
                gross_exposure,
                &bar_labels,
            ));
        }
    }

    assets.sort_unstable();
    PortfolioIntegratorLedger { rows, assets }
}

fn integrator_row_from_frame(
    frame: &PortfolioTrackingFrame,
    bar_labels: &Option<Vec<String>>,
) -> IntegratorLedgerRow {
    let timestamp_label = bar_labels
        .as_ref()
        .and_then(|labels| labels.get(frame.timestamp as usize))
        .cloned()
        .unwrap_or_else(|| format_tick_label(frame.timestamp as usize));
    let risk_modified = (frame.closure_raw_weight - frame.altered_portfolio_weight.abs()).abs()
        > 1e-9;
    IntegratorLedgerRow {
        timestamp_label,
        asset_id: frame.asset_id.clone(),
        closure_raw_weight: frame.closure_raw_weight,
        altered_portfolio_weight: frame.altered_portfolio_weight,
        nominal_price: frame.current_nominal_price,
        calculated_units: frame.calculated_units,
        investment_return: frame.investment_return,
        risk_modified,
        is_portfolio_summary: false,
    }
}

fn integrator_portfolio_row(
    bar_index: usize,
    nav: f64,
    bar_return: f64,
    gross_exposure: f64,
    bar_labels: &Option<Vec<String>>,
) -> IntegratorLedgerRow {
    let timestamp_label = bar_labels
        .as_ref()
        .and_then(|labels| labels.get(bar_index))
        .cloned()
        .unwrap_or_else(|| format_tick_label(bar_index));
    IntegratorLedgerRow {
        timestamp_label,
        asset_id: PORTFOLIO_LEDGER_ASSET.to_string(),
        closure_raw_weight: 1.0,
        altered_portfolio_weight: gross_exposure,
        nominal_price: nav,
        calculated_units: f64::NAN,
        investment_return: bar_return,
        risk_modified: false,
        is_portfolio_summary: true,
    }
}

pub fn ledger_csv_content(rows: &[IntegratorLedgerRow]) -> String {
    let mut out = String::from(
        "Date,Asset ID,Raw Closure Weight,Altered Portfolio Weight,Asset Nominal Price,Clipped Units,Investment Return\n",
    );
    for row in rows {
        out.push_str(&format!(
            "{},{},{:.8},{:.8},{:.4},{:.6},{:.8}\n",
            csv_escape(&row.timestamp_label),
            csv_escape(&row.asset_id),
            row.closure_raw_weight,
            row.altered_portfolio_weight,
            row.nominal_price,
            row.calculated_units,
            row.investment_return,
        ));
    }
    out
}

fn csv_escape(value: &str) -> String {
    if value.contains(',') || value.contains('"') || value.contains('\n') {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

fn format_weight(value: f64) -> String {
    if !value.is_finite() {
        "—".to_string()
    } else {
        format!("{:.4}", value)
    }
}

fn format_price(value: f64) -> String {
    if !value.is_finite() || value <= 0.0 {
        "—".to_string()
    } else if value >= 1000.0 {
        format!("{value:.2}")
    } else {
        format!("{value:.4}")
    }
}

fn format_units(value: f64) -> String {
    if !value.is_finite() {
        "—".to_string()
    } else {
        format!("{value:.4}")
    }
}

fn format_nav(value: f64) -> String {
    if !value.is_finite() {
        "—".to_string()
    } else {
        format!("${value:.2}")
    }
}

pub trait IntegratorLedgerHost: Sized + 'static {
    fn set_integrator_ledger_filter(
        &mut self,
        filter: IntegratorLedgerFilter,
        cx: &mut Context<Self>,
    );

    fn export_integrator_ledger_csv(&mut self, cx: &mut Context<Self>);
}

pub fn render_integrator_ledger_spreadsheet<H: IntegratorLedgerHost>(
    ledger: Arc<PortfolioIntegratorLedger>,
    filter: IntegratorLedgerFilter,
    view: Entity<H>,
) -> impl IntoElement {
    let filtered = ledger.filtered_indices(&filter);
    let row_count = filtered.len();
    let filtered_indices = filtered.clone();
    let ledger_for_list = ledger.clone();
    let filter_for_export = filter.clone();

    div()
        .flex_col()
        .gap_2()
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap_2()
                .child(
                    div()
                        .text_size(px(9.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .font_family("monospace")
                        .text_color(rgb(TEXT))
                        .child("Integrator Ledger"),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .child(render_filter_chips(
                            ledger.clone(),
                            filter.clone(),
                            view.clone(),
                        ))
                        .child(
                            Button::new("export-integrator-ledger")
                                .label("Export CSV")
                                .on_click({
                                    let view = view.clone();
                                    move |_, _, cx| {
                                        view.update(cx, |host, cx| {
                                            host.export_integrator_ledger_csv(cx);
                                        });
                                    }
                                }),
                        ),
                ),
        )
        .child(
            div()
                .text_size(px(8.0))
                .font_family("monospace")
                .text_color(rgb(TEXT_MUTED))
                .child(format!(
                    "{} rows · filter: {}",
                    row_count,
                    filter_for_export.label()
                )),
        )
        .child(
            div()
                .flex_col()
                .h(px(240.0))
                .min_h(px(180.0))
                .rounded_md()
                .border_1()
                .border_color(rgb(BORDER))
                .bg(rgb(0x0f0f12))
                .overflow_hidden()
                .child(render_ledger_header_row())
                .child(if row_count == 0 {
                    div()
                        .flex_1()
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_size(px(10.0))
                        .font_family("monospace")
                        .text_color(rgb(TEXT_MUTED))
                        .child("No integrator records for this filter.")
                        .into_any_element()
                } else {
                    let body_rows: Vec<_> = filtered_indices
                        .iter()
                        .enumerate()
                        .map(|(visible_ix, row_ix)| {
                            render_ledger_data_row(&ledger_for_list.rows[*row_ix], visible_ix)
                        })
                        .collect();
                    div()
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scrollbar()
                        .child(
                            div()
                                .flex_col()
                                .min_w(px(532.0))
                                .children(body_rows),
                        )
                        .into_any_element()
                }),
        )
}

fn render_filter_chips<H: IntegratorLedgerHost>(
    ledger: Arc<PortfolioIntegratorLedger>,
    active: IntegratorLedgerFilter,
    view: Entity<H>,
) -> impl IntoElement {
    let mut row = div().flex().flex_wrap().gap_1().max_w(px(180.0));
    row = row.child(filter_chip(
        0,
        "All",
        active == IntegratorLedgerFilter::AllAssets,
        {
            let view = view.clone();
            move |cx| {
                view.update(cx, |host, cx| {
                    host.set_integrator_ledger_filter(IntegratorLedgerFilter::AllAssets, cx);
                });
            }
        },
    ));
    row = row.child(filter_chip(
        1,
        "Risk Δ",
        active == IntegratorLedgerFilter::RiskModifiedOnly,
        {
            let view = view.clone();
            move |cx| {
                view.update(cx, |host, cx| {
                    host.set_integrator_ledger_filter(
                        IntegratorLedgerFilter::RiskModifiedOnly,
                        cx,
                    );
                });
            }
        },
    ));
    row = row.child(filter_chip(
        2,
        "NAV",
        active == IntegratorLedgerFilter::PortfolioNav,
        {
            let view = view.clone();
            move |cx| {
                view.update(cx, |host, cx| {
                    host.set_integrator_ledger_filter(IntegratorLedgerFilter::PortfolioNav, cx);
                });
            }
        },
    ));
    for (index, asset) in ledger.assets.iter().enumerate() {
        let asset_id = asset.clone();
        let chip_label = asset_id.clone();
        let is_active = matches!(
            &active,
            IntegratorLedgerFilter::Asset(symbol) if symbol == asset
        );
        let view = view.clone();
        row = row.child(filter_chip(index + 3, &chip_label, is_active, move |cx| {
            view.update(cx, |host, cx| {
                host.set_integrator_ledger_filter(
                    IntegratorLedgerFilter::Asset(asset_id.clone()),
                    cx,
                );
            });
        }));
    }
    row
}

fn filter_chip(
    id: usize,
    label: &str,
    active: bool,
    on_click: impl Fn(&mut App) + 'static,
) -> impl IntoElement {
    let label_text = label.to_string();
    Button::new(("integrator-filter", id))
        .label(label_text)
        .on_click(move |_, _, cx| on_click(cx))
        .when(active, |button| button.selected(true))
}

fn render_ledger_header_row() -> impl IntoElement {
    div()
        .flex()
        .flex_none()
        .h(px(24.0))
        .bg(rgb(HEADER_BG))
        .border_b_1()
        .border_color(rgb(BORDER))
        .children(header_cells(&[
            ("Date", COL_DATE),
            ("Asset", COL_ASSET),
            ("Raw Wt", COL_RAW),
            ("Adj Wt", COL_ALTERED),
            ("Nominal", COL_PRICE),
            ("Units", COL_UNITS),
            ("Return", COL_RETURN),
        ]))
}

fn header_cells(columns: &[(&str, Pixels)]) -> Vec<impl IntoElement> {
    columns
        .iter()
        .map(|(label, width)| {
            div()
                .flex_none()
                .w(*width)
                .px_1()
                .text_size(px(8.0))
                .font_weight(FontWeight::SEMIBOLD)
                .font_family("monospace")
                .text_color(rgb(TEXT_MUTED))
                .child(label.to_string())
        })
        .collect()
}

fn render_ledger_data_row(row: &IntegratorLedgerRow, visible_ix: usize) -> impl IntoElement {
    let bg = if row.is_portfolio_summary {
        ROW_PORTFOLIO
    } else if row.risk_modified {
        ROW_RISK
    } else if visible_ix % 2 == 0 {
        ROW_A
    } else {
        ROW_B
    };
    let return_color = if row.investment_return >= 0.0 {
        rgb(0x10b981)
    } else {
        rgb(0xf87171)
    };
    let asset_color = if row.is_portfolio_summary {
        rgb(TEXT_PORTFOLIO)
    } else {
        rgb(TEXT_ACCENT)
    };
    let raw_label = if row.is_portfolio_summary {
        "1.0000".to_string()
    } else {
        format_weight(row.closure_raw_weight)
    };
    let units_label = if row.is_portfolio_summary {
        "—".to_string()
    } else {
        format_units(row.calculated_units)
    };
    let nominal_label = if row.is_portfolio_summary {
        format_nav(row.nominal_price)
    } else {
        format_price(row.nominal_price)
    };

    div()
        .flex()
        .flex_none()
        .h(px(ROW_HEIGHT))
        .bg(rgb(bg))
        .border_b_1()
        .border_color(rgb(BORDER))
        .child(cell(&row.timestamp_label, COL_DATE, rgb(TEXT)))
        .child(cell(&row.asset_id, COL_ASSET, asset_color))
        .child(cell(&raw_label, COL_RAW, rgb(TEXT)))
        .child(cell(
            &format_weight(row.altered_portfolio_weight),
            COL_ALTERED,
            if row.risk_modified {
                rgb(0xf59e0b)
            } else {
                rgb(TEXT)
            },
        ))
        .child(cell(&nominal_label, COL_PRICE, rgb(TEXT)))
        .child(cell(&units_label, COL_UNITS, rgb(TEXT)))
        .child(cell(
            &format_percent_signed(row.investment_return),
            COL_RETURN,
            return_color,
        ))
}

fn cell(label: &str, width: Pixels, color: Rgba) -> impl IntoElement {
    div()
        .flex_none()
        .w(width)
        .px_1()
        .text_size(px(9.0))
        .font_family("monospace")
        .text_color(color)
        .truncate()
        .child(label.to_string())
}
