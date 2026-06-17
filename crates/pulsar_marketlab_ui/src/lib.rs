//! GPUI workstation shell for MarketLab.

pub mod node_inline_controls;
pub mod otl_code_editor;
pub mod otl_inspector;
pub mod theme;
pub mod workspace;

pub use node_inline_controls::{
    dcc_multiline_input, dcc_singleline_input, node_dropdown_trigger, NodeNumberInput,
};
pub use otl_code_editor::{new_otl_code_editor_state, render_otl_code_editor, OTL_CODE_EDITOR_LANGUAGE};
pub use theme::buttons::{
    chip_button_variant, dcc_chip_button, dcc_secondary_button, dcc_toolbar_button,
    secondary_button_variant, toolbar_button_variant,
};
pub use otl_inspector::{dcc_otl_script_input, init as init_otl_inspector, CommitOtlScript};
pub use theme::{apply_chart_candle_accents, chrome_color, color_with_alpha};
