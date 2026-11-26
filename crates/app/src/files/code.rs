//! The viewer's editor, and the languages it colors.

use std::path::PathBuf;
use std::sync::Once;

use detached_core::api;
use gpui_kit::base::TestSupportExt;
use gpui_kit::component::highlighter::{LanguageConfig, LanguageRegistry};
use gpui_kit::component::input::{Editor, EditorState, InputEvent, Position, RopeExt};
use gpui_kit::{
    App, AppContext, Context, Entity, InteractiveElement, IntoElement, ParentElement, Render,
    StatefulInteractiveElement, Styled, Subscription, Task, Window, div, point, px,
};

use crate::dock::ViewPosition;
use crate::keymap;
use crate::ui::{self, theme};
use crate::util::base_name;
use kernel::activity::Activity;
use kernel::blocking;

pub(super) struct Code {
    editor: Entity<EditorState>,
    /// A line to center once the editor is laid out.
    center: Option<u32>,
    restored: Option<ViewPosition>,
    editable: bool,
    dirty: bool,
    /// The text on disk as last read or written.
    disk: String,
    /// A newer text on disk that the edited buffer has not taken.
    external: Option<String>,
    _change: Subscription,
}

impl Code {
    pub(super) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let editor = cx.new(|cx| EditorState::new(window, cx));
        let change = cx.subscribe(&editor, |this, _, e: &InputEvent, cx| {
            if matches!(e, InputEvent::Change) && this.editable {
                this.dirty = true;
                cx.notify();
            }
        });
        Code {
            editor,
            center: None,
            restored: None,
            editable: false,
            dirty: false,
            disk: String::new(),
            external: None,
            _change: change,
        }
    }

    pub(super) fn set_editable(&mut self, editable: bool, cx: &mut Context<Self>) {
        self.editable = editable;
        cx.notify();
    }

    pub(super) fn unsaved(&self) -> bool {
        self.dirty
    }

    /// Writes the buffer to `file`; false when it could not, which it says.
    pub(super) fn save(&mut self, file: PathBuf, cx: &mut Context<Self>) -> Task<bool> {
        if !self.dirty {
            return Task::ready(true);
        }
        let text = self.text(cx);
        let body = text.clone();
        let write = blocking::run(cx, move || {
            api::write_text(&file, &body).map_err(|e| e.to_string())
        });
        cx.spawn(async move |this, cx| match write.await {
            Ok(written) => this
                .update(cx, |c, cx| {
                    c.dirty = c.text(cx) != text;
                    c.disk = written;
                    c.external = None;
                    cx.notify();
                })
                .is_ok(),
            Err(e) => {
                cx.update(|cx| {
                    let said = format!("Could not save: {e}");
                    Activity::global(cx).update(cx, |a, cx| a.say(said, true, cx));
                });
                false
            }
        })
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
        if self.dirty {
            let same = *self.editor.read(cx).text() == text;
            if same {
                (self.dirty, self.disk) = (false, text.to_string());
            }
            self.external = (!same && text != self.disk).then(|| text.to_string());
            cx.notify();
            return;
        }
        self.disk = text.to_string();
        self.external = None;
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

    pub(super) fn reveal(
        &mut self,
        line: u32,
        col: u32,
        focus: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let at = Position::new(line.saturating_sub(1), col.saturating_sub(1));
        self.editor.update(cx, |e, cx| {
            if focus {
                e.set_cursor_position(at, window, cx);
            } else {
                let offset = e.text().position_to_offset(&at);
                e.set_selected_range(offset..offset, cx);
            }
        });
        self.center = Some(line.max(1));
        self.restored = None;
        cx.notify();
    }

    pub(super) fn position(&self, cx: &App) -> ViewPosition {
        if let Some(at) = self.restored {
            return at;
        }
        let e = self.editor.read(cx);
        let at = e.cursor_position();
        let scroll = e.scroll_offset();
        ViewPosition {
            line: at.line,
            column: at.character,
            x: scroll.x.as_f32(),
            y: scroll.y.as_f32(),
        }
    }

    pub(super) fn restore(&mut self, at: ViewPosition, cx: &mut Context<Self>) {
        self.center = None;
        self.restored = Some(at);
        cx.notify();
    }

    pub(super) fn text(&self, cx: &App) -> String {
        self.editor.read(cx).text().to_string()
    }

    pub(super) fn focus(&self, window: &mut Window, cx: &mut App) {
        self.editor.update(cx, |e, cx| e.focus(window, cx));
    }
}

impl Render for Code {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(at) = self.restored
            && self.editor.read(cx).line_height().is_some()
        {
            self.restored = None;
            self.editor.update(cx, |e, cx| {
                let i = e
                    .text()
                    .position_to_offset(&Position::new(at.line, at.column));
                e.set_selected_range(i..i, cx);
                e.set_scroll_offset(point(px(at.x), px(at.y)), cx);
            });
        } else if self.restored.is_some() {
            window.request_animation_frame();
        }
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
        let bar = self.external.is_some().then(|| {
            let t = theme(cx);
            let text = "Changed on disk";
            let reload = ui::button("code.reload", "Reload", cx)
                .test_support()
                .on_click(cx.listener(|this, _, window, cx| {
                    let text = this.external.take().unwrap_or_default();
                    this.editor
                        .update(cx, |e, cx| e.set_value(text.clone(), window, cx));
                    (this.dirty, this.disk) = (false, text);
                    cx.notify();
                }));
            let keep = ui::button("code.keep", "Keep mine", cx)
                .test_support()
                .on_click(cx.listener(|this, _, _, cx| {
                    this.disk = this.external.take().unwrap_or_default();
                    cx.notify();
                }));
            div()
                .id("code.changed")
                .test_support()
                .aria_label(text)
                .flex()
                .flex_none()
                .items_center()
                .gap(px(8.))
                .px(px(16.))
                .py(px(6.))
                .bg(t.conflict.opacity(0.14))
                .border_b_1()
                .border_color(t.line)
                .child(div().flex_1().child(text))
                .child(reload)
                .child(keep)
        });
        div()
            .id("viewer.code")
            .key_context(keymap::VIEWER)
            .flex()
            .flex_col()
            .size_full()
            .children(bar)
            .child(
                div().flex_1().min_h_0().child(
                    Editor::new(&self.editor)
                        .readonly(!self.editable)
                        .bordered(false)
                        .h_full(),
                ),
            )
    }
}

/// Monaco's languages the kit has a grammar for, by lower-cased file name.
const NAMES: &[(&str, &str)] = &[("dockerfile", "dockerfile"), ("jakefile", "javascript")];

/// By extension; `.svelte`, `.vue` and `.astro` as HTML.
const EXTENSIONS: &[(&str, &[&str])] = &[
    ("bash", &[".sh", ".bash"]),
    ("c", &[".c", ".h"]),
    ("cpp", &[".cpp", ".cc", ".cxx", ".hpp", ".hh", ".hxx"]),
    ("csharp", &[".cs", ".csx", ".cake"]),
    ("css", &[".css", ".scss"]),
    ("dockerfile", &[".dockerfile"]),
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
    ("rust", &[".rs", ".rlib"]),
    ("sql", &[".sql"]),
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

/// Monaco's aliases that are neither a name above nor an extension.
const ALIASES: &[(&str, &str)] = &[("shell", "bash"), ("c++", "cpp"), ("c#", "csharp")];

/// A code fence's language as the kit names it: by name, alias or extension; None for plain text.
pub(super) fn fence_language(tag: &str) -> Option<&'static str> {
    let tag = tag.to_lowercase();
    let lang = EXTENSIONS
        .iter()
        .map(|(lang, _)| *lang)
        .find(|lang| *lang == tag)
        .or_else(|| ALIASES.iter().find(|(a, _)| *a == tag).map(|(_, l)| *l))
        .unwrap_or_else(|| language_for(&format!("x.{tag}")));
    (lang != "plaintext").then_some(lang)
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
