//! The viewer's read-only editor, and the languages it colors.

use std::sync::Once;

use gpui_kit::component::highlighter::{LanguageConfig, LanguageRegistry};
use gpui_kit::component::input::{Editor, EditorState, Position, RopeExt};
use gpui_kit::{
    App, AppContext, Context, Entity, InteractiveElement, IntoElement, ParentElement, Render,
    Styled, Window, div, point, px,
};

use crate::keymap;
use crate::util::base_name;

pub(super) struct Code {
    editor: Entity<EditorState>,
    /// A line to center once the editor is laid out.
    center: Option<u32>,
}

impl Code {
    pub(super) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let editor = cx.new(|cx| {
            let mut e = EditorState::new(window, cx);
            e.set_readonly(true, cx);
            e
        });
        Code {
            editor,
            center: None,
        }
    }

    /// Shows `text` colored for `path`; the same text changes nothing. `keep` holds the cursor
    /// and the scroll when it changes.
    pub(super) fn set(
        &mut self,
        path: &str,
        text: &str,
        keep: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let lang = language_for(path);
        self.editor.update(cx, |e, cx| {
            if e.language_name() != lang {
                e.set_highlighter(lang, cx);
            }
            if *e.text() == text {
                return;
            }
            let (at, scroll) = (e.cursor_position(), e.scroll_offset());
            e.set_value(text.to_string(), window, cx);
            if keep {
                let i = e.text().position_to_offset(&at);
                e.set_selected_range(i..i, cx);
                e.set_scroll_offset(scroll, cx);
            }
        });
    }

    /// Puts the cursor on `line` at `col` (from 1), centers the line and focuses the editor.
    pub(super) fn reveal(
        &mut self,
        line: u32,
        col: u32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let at = Position::new(line.saturating_sub(1), col.saturating_sub(1));
        self.editor
            .update(cx, |e, cx| e.set_cursor_position(at, window, cx));
        self.center = Some(line.max(1));
        cx.notify();
    }

    /// The cursor's line, from 1.
    pub(super) fn line(&self, cx: &App) -> u32 {
        self.editor.read(cx).cursor_position().line + 1
    }

    pub(super) fn focus(&self, window: &mut Window, cx: &mut App) {
        self.editor.update(cx, |e, cx| e.focus(window, cx));
    }
}

impl Render for Code {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(line) = self.center {
            let e = self.editor.read(cx);
            // The kit scrolls the cursor just into view; Monaco centered it.
            match e.line_height() {
                Some(h) => {
                    let view = e.input_bounds().size.height;
                    let top = (h * (line - 1) as f32 - (view - h) / 2.).max(px(0.));
                    let x = e.scroll_offset().x;
                    self.editor
                        .update(cx, |e, cx| e.set_scroll_offset(point(x, -top), cx));
                    self.center = None;
                }
                None => window.request_animation_frame(),
            }
        }
        div()
            .id("viewer.code")
            .key_context(keymap::VIEWER)
            .size_full()
            .child(Editor::new(&self.editor).bordered(false).h_full())
    }
}

/// Monaco's languages the kit has a grammar for, by lower-cased file name.
const NAMES: &[(&str, &str)] = &[
    ("dockerfile", "dockerfile"),
    ("jakefile", "javascript"),
    ("rakefile", "ruby"),
    ("gemfile", "ruby"),
];

/// By extension; `.svelte`, `.vue` and `.astro` as HTML, as the Tauri client did.
const EXTENSIONS: &[(&str, &[&str])] = &[
    ("bash", &[".sh", ".bash"]),
    ("c", &[".c", ".h"]),
    ("cpp", &[".cpp", ".cc", ".cxx", ".hpp", ".hh", ".hxx"]),
    ("csharp", &[".cs", ".csx", ".cake"]),
    ("css", &[".css", ".scss"]),
    ("dockerfile", &[".dockerfile"]),
    ("elixir", &[".ex", ".exs"]),
    ("go", &[".go"]),
    ("graphql", &[".graphql", ".gql"]),
    (
        "html",
        &[
            ".html", ".htm", ".shtml", ".xhtml", ".mdoc", ".jsp", ".asp", ".aspx", ".jshtm",
            ".svelte", ".vue", ".astro",
        ],
    ),
    ("java", &[".java", ".jav"]),
    ("javascript", &[".js", ".es6", ".jsx", ".mjs", ".cjs"]),
    (
        "json",
        &[
            ".json",
            ".bowerrc",
            ".jshintrc",
            ".jscsrc",
            ".eslintrc",
            ".babelrc",
            ".har",
        ],
    ),
    ("kotlin", &[".kt", ".kts"]),
    ("lua", &[".lua"]),
    (
        "markdown",
        &[
            ".md",
            ".markdown",
            ".mdown",
            ".mkdn",
            ".mkd",
            ".mdwn",
            ".mdtxt",
            ".mdtext",
        ],
    ),
    ("php", &[".php", ".php4", ".php5", ".phtml", ".ctp"]),
    ("proto", &[".proto"]),
    ("python", &[".py", ".rpy", ".pyw", ".cpy", ".gyp", ".gypi"]),
    ("ruby", &[".rb", ".rbx", ".rjs", ".gemspec", ".pp"]),
    ("rust", &[".rs", ".rlib"]),
    ("scala", &[".scala", ".sc", ".sbt"]),
    ("sql", &[".sql"]),
    ("swift", &[".swift"]),
    ("tsx", &[".tsx"]),
    ("typescript", &[".ts", ".cts", ".mts"]),
    ("yaml", &[".yaml", ".yml"]),
];

/// The kit's name for `path`'s language: by file name, then by the last extension (none when the
/// name starts with the dot), else plain text.
pub(super) fn language_for(path: &str) -> &'static str {
    static DOCKERFILE: Once = Once::new();
    DOCKERFILE.call_once(register_dockerfile);
    let name = base_name(path).to_lowercase();
    let ext = name.rfind('.').filter(|&i| i > 0).map(|i| &name[i..]);
    let by_ext = || {
        let ext = ext?;
        EXTENSIONS.iter().find(|(_, exts)| exts.contains(&ext))
    };
    NAMES
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, lang)| *lang)
        .or_else(|| by_ext().map(|(lang, _)| *lang))
        .unwrap_or("plaintext")
}

/// The kit ships no Dockerfile grammar.
fn register_dockerfile() {
    use tree_sitter_containerfile as ts;
    let config = LanguageConfig::new(
        "dockerfile",
        ts::LANGUAGE.into(),
        vec!["bash".into(), "json".into()],
        ts::HIGHLIGHTS_QUERY,
        ts::INJECTIONS_QUERY,
        "",
    );
    LanguageRegistry::singleton().register("dockerfile", &config);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uses_file_names_then_extensions_then_plaintext() {
        assert_eq!(language_for("src/App.ts"), "typescript");
        assert_eq!(language_for("docker/Dockerfile"), "dockerfile");
        assert_eq!(language_for("App.svelte"), "html");
        assert_eq!(language_for(".env"), "plaintext");
        assert_eq!(language_for("lib/x.MJS"), "javascript");
        assert!(
            LanguageRegistry::singleton()
                .language("dockerfile")
                .is_some()
        );
    }
}
