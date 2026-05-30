//! Dark-themed GPUI code editor shell for OTL / OSL C-style scripts.

use gpui::*;
use gpui_component::input::{Input, InputState, TabSize};
use gpui_component::v_flex;

/// Tree-sitter grammar for OSL-style `void fn(...)` scripts.
///
/// Uses `c` rather than `cpp`: the bundled `tree-sitter-cpp` highlight query omits
/// comments, primitive keywords (`void`, `int`, `float`), and most C-style rules.
pub const OTL_CODE_EDITOR_LANGUAGE: &str = "c";

/// Create a searchable multi-line code editor with line numbers and C/C++ highlighting.
pub fn new_otl_code_editor_state(window: &mut Window, cx: &mut App) -> Entity<InputState> {
    cx.new(|cx| {
        InputState::new(window, cx)
            .code_editor(OTL_CODE_EDITOR_LANGUAGE)
            .multi_line(true)
            .line_number(true)
            .searchable(true)
            .soft_wrap(false)
            .rows(24)
            .tab_size(TabSize {
                tab_size: 4,
                hard_tabs: false,
            })
    })
}

/// Render a full-height code editor using the GPUI Component code-editor mode.
pub fn render_otl_code_editor(input: &Entity<InputState>) -> impl IntoElement {
    v_flex()
        .id("otl-code-editor-shell")
        .flex_1()
        .min_h_0()
        .min_w_0()
        .size_full()
        .font_family("monospace")
        .text_size(px(11.0))
        .child(Input::new(input).appearance(true).bordered(false).focus_bordered(false).h_full())
}

#[cfg(test)]
mod tests {
    use gpui_component::highlighter::{HighlightTheme, SyntaxHighlighter};
    use gpui_component::input::Rope;

    #[test]
    fn otl_c_syntax_highlighter_assigns_comment_color() {
        use gpui_component::highlighter::LanguageRegistry;

        assert!(
            LanguageRegistry::singleton().language("c").is_some(),
            "c grammar should be registered"
        );

        let code = "// comment\nvoid adaptive_trigger(float x) {}";
        let rope = Rope::from_str(code);
        let mut highlighter = SyntaxHighlighter::new("c");
        highlighter.update(None, &rope);

        let theme = HighlightTheme::default_dark();
        let styles = highlighter.styles(&(0..code.len()), &theme);
        let colored = styles
            .iter()
            .filter(|(_, style)| style.color.is_some())
            .count();

        assert!(
            colored > 0,
            "expected syntax colors for OSL/c, got {colored} colored spans in {styles:?}"
        );
    }

    #[test]
    fn cpp_highlight_query_is_minimal_without_comment_rules() {
        let code = "// comment\nvoid adaptive_trigger() {}";
        let rope = Rope::from_str(code);
        let mut highlighter = SyntaxHighlighter::new("cpp");
        highlighter.update(None, &rope);

        let theme = HighlightTheme::default_dark();
        let styles = highlighter.styles(&(0..code.len()), &theme);
        let colored = styles
            .iter()
            .filter(|(_, style)| style.color.is_some())
            .count();

        assert_eq!(
            colored, 0,
            "document why OTL avoids cpp: tree-sitter-cpp highlights.scm lacks comment rules"
        );
    }
}
