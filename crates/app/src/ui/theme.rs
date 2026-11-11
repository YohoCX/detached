//! Colors from theme files, the bundled ones and the user's `themes/*.json`, light and dark
//! following the system. A file only needs the tokens it changes; the rest come from `extends`.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, SystemTime};

use gpui_kit::component::Theme as Kit;
use gpui_kit::{App, BoxShadow, Global, Hsla, Pixels, SharedString, Window, px, rgba};
use serde_json::{Map, Value};

use crate::util::Status;
use kernel::activity::Activity;
use kernel::blocking;
use kernel::ui_state;
use kernel::workspace::Workspace;

pub const DEFAULT: &str = "Fleet";
const BUNDLED: [&str; 2] = [
    include_str!("../../themes/fleet.json"),
    include_str!("../../themes/classic.json"),
];
const FONTS: [&[u8]; 8] = [
    include_bytes!("../../fonts/Inter-Regular.ttf"),
    include_bytes!("../../fonts/Inter-Medium.ttf"),
    include_bytes!("../../fonts/Inter-SemiBold.ttf"),
    include_bytes!("../../fonts/Inter-Bold.ttf"),
    include_bytes!("../../fonts/Inter-ExtraBold.ttf"),
    include_bytes!("../../fonts/JetBrainsMono-Regular.ttf"),
    include_bytes!("../../fonts/JetBrainsMono-Bold.ttf"),
    include_bytes!("../../fonts/SymbolsNerdFontMono-Regular.ttf"),
];
const KEY: &str = "theme.name";
/// How often the themes folder is looked at for edits.
const POLL: Duration = Duration::from_millis(500);

macro_rules! tokens {
    ($($color:ident),* ; $($shadow:ident),* ; $($list:ident: $n:literal),*) => {
        pub struct Theme {
            $(pub $color: Hsla,)*
            $(pub $shadow: Vec<BoxShadow>,)*
            $(pub $list: [Hsla; $n],)*
        }

        fn parse(m: &Map<String, Value>) -> Result<Theme, String> {
            Ok(Theme {
                $($color: color(m, stringify!($color))?,)*
                $($shadow: shadows(m, stringify!($shadow))?,)*
                $($list: colors(m, stringify!($list))?,)*
            })
        }
    };
}

tokens!(
    ground, panel, panel_line, code, fg, muted, line, accent, accent_fg, primary, primary_fg,
    hover, selected, selected_ring, selected_inactive, hit, popup, scrim, input, error, warning,
    success, added, modified, deleted, conflict, untracked,
    // Monaco's cursor line and text selection.
    line_highlight, text_selection;
    panel_shadow, shadow;
    // The 16 terminal colors, for job logs and the terminal; the log graph's lanes.
    ansi: 16, lanes: 8
);

/// The gap between panes and around the window.
pub const GAP: Pixels = px(10.);

pub mod radius {
    use gpui_kit::{Pixels, px};

    pub const PILL: Pixels = px(999.);
    pub const DIALOG: Pixels = px(20.);
    pub const PALETTE: Pixels = px(18.);
    pub const GLASS: Pixels = px(16.);
    pub const PANEL: Pixels = px(14.);
    /// The palette's query field, account rows.
    pub const FIELD: Pixels = px(12.);
    /// Strip buttons, top bar widgets, sidebar rows.
    pub const WIDGET: Pixels = px(10.);
    /// Inputs and list items.
    pub const ITEM: Pixels = px(9.);
    pub const ICON_BUTTON: Pixels = px(8.);
    pub const SEGMENT: Pixels = px(7.);
    pub const KBD: Pixels = px(6.);
    pub const LETTER: Pixels = px(5.);
    pub const BADGE: Pixels = px(4.);
}

pub mod text {
    use gpui_kit::{Pixels, px};

    pub const BODY: Pixels = px(13.);
    /// Status bar, notices, mono text.
    pub const SMALL: Pixels = px(12.);
    /// Section labels and key hints.
    pub const LABEL: Pixels = px(11.);
    pub const TITLE: Pixels = px(14.);
    /// Change status letters.
    pub const LETTER: Pixels = px(10.);
    /// A dialog's title.
    pub const HEADING: Pixels = px(15.);
}

/// A markdown file's column.
pub mod doc {
    use gpui_kit::{Pixels, px};

    pub const WIDTH: Pixels = px(880.);
    pub const PAD_TOP: Pixels = px(24.);
    pub const PAD_X: Pixels = px(32.);
    pub const PAD_BOTTOM: Pixels = px(48.);
    pub const BODY: Pixels = px(14.);
    pub const LINE_HEIGHT: f32 = 1.6;
    pub const H1: Pixels = px(26.);
    pub const H2: Pixels = px(20.);
    pub const H3: Pixels = px(16.);
    pub const CODE: Pixels = px(12.5);
}

fn hex(s: &str) -> Option<Hsla> {
    let h = s.strip_prefix('#')?;
    let n = u32::from_str_radix(h, 16).ok()?;
    let n = match h.len() {
        6 => (n << 8) | 0xff,
        8 => n,
        _ => return None,
    };
    Some(rgba(n).into())
}

fn color(m: &Map<String, Value>, key: &str) -> Result<Hsla, String> {
    let v = m.get(key).ok_or(format!("{key} is missing"))?;
    v.as_str()
        .and_then(hex)
        .ok_or(format!("{key}: {v} is not #rrggbb or #rrggbbaa"))
}

fn colors<const N: usize>(m: &Map<String, Value>, key: &str) -> Result<[Hsla; N], String> {
    let v = m
        .get(key)
        .and_then(Value::as_array)
        .ok_or(format!("{key} is not a list"))?;
    let all: Option<Vec<Hsla>> = v.iter().map(|c| c.as_str().and_then(hex)).collect();
    let all = all.ok_or(format!("{key}: a color is not #rrggbb or #rrggbbaa"))?;
    all.try_into()
        .map_err(|_| format!("{key} needs {N} colors"))
}

fn shadows(m: &Map<String, Value>, key: &str) -> Result<Vec<BoxShadow>, String> {
    let v = m
        .get(key)
        .and_then(Value::as_array)
        .ok_or(format!("{key} is not a list"))?;
    v.iter()
        .map(|s| {
            let n = |k: &str| s[k].as_f64().unwrap_or(0.) as f32;
            let color = s["color"].as_str().and_then(hex);
            let color = color.ok_or(format!(
                "{key}: a shadow's color is not #rrggbb or #rrggbbaa"
            ))?;
            let shadow = BoxShadow::new(px(n("x")), px(n("y")), color)
                .blur_radius(px(n("blur")))
                .spread_radius(px(n("spread")));
            Ok(if s["inset"].as_bool() == Some(true) {
                shadow.inset()
            } else {
                shadow
            })
        })
        .collect()
}

/// A theme file's light and dark tokens over those of the theme it extends.
fn pair(v: &Value, bases: &BTreeMap<String, Value>) -> Result<(Theme, Theme), String> {
    let base = v["extends"].as_str().unwrap_or(DEFAULT);
    let base = bases
        .get(base)
        .ok_or(format!("extends {base}, which is not a bundled theme"))?;
    let side = |mode: &str| {
        let mut m = base[mode].as_object().cloned().unwrap_or_default();
        if let Some(own) = v[mode].as_object() {
            m.extend(own.clone());
        }
        parse(&m).map_err(|e| format!("{mode}.{e}"))
    };
    Ok((side("light")?, side("dark")?))
}

fn bundled() -> BTreeMap<String, Value> {
    BUNDLED
        .iter()
        .filter_map(|t| serde_json::from_str::<Value>(t).ok())
        .map(|v| (v["name"].as_str().unwrap_or_default().to_string(), v))
        .collect()
}

/// When each user theme file last changed.
type Stamp = Vec<(PathBuf, Option<SystemTime>, u64)>;

fn stamp(dir: &Path) -> Stamp {
    let mut files: Stamp = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .map(|p| {
            let m = p.metadata().ok();
            let len = m.as_ref().map_or(0, |m| m.len());
            (p, m.and_then(|m| m.modified().ok()), len)
        })
        .collect();
    files.sort_by(|a, b| a.0.cmp(&b.0));
    files
}

struct Loaded {
    light: Theme,
    dark: Theme,
    /// The UI and the mono family; None keeps the system's.
    font: Option<SharedString>,
    mono: Option<SharedString>,
    /// The user's file it came from.
    file: Option<PathBuf>,
}

type Found = Result<Loaded, String>;

/// Every theme's name, bundled ones first, and theme `name` with its file or why it can't be had.
/// `last` is the file it came from before: if that no longer parses, that is why.
fn find(dir: &Path, name: &str, last: Option<&Path>) -> (Vec<String>, Found) {
    let bases = bundled();
    let mut names: Vec<String> = BUNDLED
        .iter()
        .filter_map(|t| serde_json::from_str::<Value>(t).ok())
        .filter_map(|v| v["name"].as_str().map(String::from))
        .collect();
    let mut found = bases.get(name).map(|v| (name.to_string(), v.clone(), None));
    let mut broken = None;
    for (path, ..) in stamp(dir) {
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();
        let file = format!(
            "themes/{}",
            path.file_name()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
        );
        let parsed = std::fs::read_to_string(&path)
            .map_err(|e| e.to_string())
            .and_then(|t| serde_json::from_str::<Value>(&t).map_err(|e| e.to_string()));
        match parsed {
            Ok(v) => {
                let own = v["name"].as_str().map_or(stem, String::from);
                if own == name {
                    found = Some((file, v, Some(path.clone())));
                }
                if !names.contains(&own) {
                    names.push(own);
                }
            }
            Err(e) if stem == name || last == Some(path.as_path()) => {
                broken = Some(format!("{file}: {e}"))
            }
            Err(_) => {}
        }
    }
    let theme = match (broken, found) {
        (Some(e), _) => Err(e),
        (None, Some((file, v, path))) => pair(&v, &bases)
            .map(|(light, dark)| {
                let font = |k: &str| v[k].as_str().map(|f| SharedString::from(f.to_string()));
                let (font, mono) = (font("font"), font("mono_font"));
                Loaded {
                    light,
                    dark,
                    font,
                    mono,
                    file: path,
                }
            })
            .map_err(|e| format!("{file}: {e}")),
        (None, None) => Err(format!("there is no theme named {name}")),
    };
    (names, theme)
}

static FALLBACK: LazyLock<(Theme, Theme)> = LazyLock::new(|| {
    let bases = bundled();
    pair(&bases[DEFAULT], &bases).expect("the bundled default theme parses")
});

/// The theme in use, leaked so `theme` can hand out `&'static`: one pair, a few KB, per change.
struct Active {
    name: String,
    names: Vec<String>,
    light: &'static Theme,
    dark: &'static Theme,
    /// The theme's families, or the system's it found at start.
    font: SharedString,
    mono: SharedString,
    system: (SharedString, SharedString),
    /// Why the chosen theme did not load, when it didn't.
    error: Option<String>,
}

impl Global for Active {}

/// Loads the chosen theme and reloads it whenever its folder changes.
pub fn init(config_dir: &Path, cx: &mut App) {
    if cx.has_global::<Active>() {
        return;
    }
    if let Err(e) = cx
        .text_system()
        .add_fonts(FONTS.map(Cow::Borrowed).to_vec())
    {
        eprintln!("detached: the bundled fonts did not load: {e}");
    }
    let dir = config_dir.join("themes");
    let name = chosen(cx);
    let mut seen = (name.clone(), stamp(&dir));
    let (names, found) = find(&dir, &name, None);
    let mut last = found.as_ref().ok().and_then(|f| f.file.clone());
    let (light, dark) = (&FALLBACK.0, &FALLBACK.1);
    let k = Kit::global(cx);
    let system = (k.font_family.clone(), k.mono_font_family.clone());
    cx.set_global(Active {
        name: name.clone(),
        names,
        light,
        dark,
        font: system.0.clone(),
        mono: system.1.clone(),
        system,
        error: None,
    });
    apply(name, found, cx);
    // Ends with the workspace, which lives as long as the app.
    let workspace = Workspace::global(cx).downgrade();
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(POLL).await;
            if workspace.upgrade().is_none() {
                return;
            }
            let d = dir.clone();
            let (name, now) = cx.update(|cx| (chosen(cx), blocking::run(cx, move || stamp(&d))));
            let now = (name.clone(), now.await);
            if now == seen {
                continue;
            }
            seen = now;
            let (d, n, l) = (dir.clone(), name.clone(), last.clone());
            let (names, found) = cx
                .update(|cx| blocking::run(cx, move || find(&d, &n, l.as_deref())))
                .await;
            if let Ok(f) = &found {
                last = f.file.clone();
            }
            cx.update(|cx| {
                cx.global_mut::<Active>().names = names;
                apply(name, found, cx);
            });
        }
    })
    .detach();
}

fn chosen(cx: &App) -> String {
    ui_state::get(KEY, cx).unwrap_or(DEFAULT.into())
}

/// Shows `found`, or keeps the last good theme and says why.
fn apply(name: String, found: Found, cx: &mut App) {
    match found {
        Ok(l) => {
            let a = cx.global_mut::<Active>();
            (a.light, a.dark) = (Box::leak(Box::new(l.light)), Box::leak(Box::new(l.dark)));
            a.font = l.font.unwrap_or(a.system.0.clone());
            a.mono = l.mono.unwrap_or(a.system.1.clone());
            a.name = name;
            a.error = None;
            sync_kit(cx);
            cx.refresh_windows();
        }
        Err(e) => {
            let text = format!("Theme {name} not loaded, keeping the last one: {e}");
            cx.global_mut::<Active>().error = Some(text.clone());
            Activity::global(cx).update(cx, |a, cx| a.say(text, true, cx));
        }
    }
}

/// Makes the user's themes folder, with a copy of the default to edit when it has none.
pub fn seed(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    if stamp(dir).is_empty() {
        let mine = BUNDLED[0].replacen(
            &format!("\"name\": \"{DEFAULT}\""),
            "\"name\": \"My theme\"",
            1,
        );
        std::fs::write(dir.join("my-theme.json"), mine)?;
    }
    Ok(())
}

/// Every theme by name and the one in use.
pub fn names(cx: &App) -> (Vec<String>, String) {
    match cx.try_global::<Active>() {
        Some(a) => (a.names.clone(), a.name.clone()),
        None => (vec![DEFAULT.into()], DEFAULT.into()),
    }
}

/// Switches to theme `name`, which the next look at the folder loads.
pub fn choose(name: &str, cx: &mut App) {
    ui_state::set(KEY, name, cx);
}

/// Why the chosen theme did not load, if it did not.
pub fn error(cx: &App) -> Option<String> {
    cx.try_global::<Active>()?.error.clone()
}

impl Theme {
    pub fn status(&self, s: Status) -> Hsla {
        match s {
            Status::Modified | Status::Renamed => self.modified,
            Status::Added => self.added,
            Status::Deleted => self.deleted,
            Status::Untracked => self.untracked,
            Status::Conflict => self.conflict,
        }
    }
}

/// Whether the kit's theme last synced to a dark appearance.
pub fn is_dark(cx: &App) -> bool {
    cx.try_global::<Kit>().is_some_and(|k| k.is_dark())
}

/// The tokens for the system appearance the kit's theme last synced to.
pub fn theme(cx: &App) -> &'static Theme {
    let dark = is_dark(cx);
    let (light, night) = match cx.try_global::<Active>() {
        Some(a) => (a.light, a.dark),
        None => (&FALLBACK.0, &FALLBACK.1),
    };
    if dark { night } else { light }
}

impl Theme {
    /// A log graph lane's color.
    pub fn lane(&self, color: u32) -> Hsla {
        self.lanes[color as usize % self.lanes.len()]
    }
}

/// The theme's monospace family, or the platform's, which the kit makes sure is installed.
pub fn mono(cx: &App) -> SharedString {
    Kit::global(cx).mono_font_family.clone()
}

/// Follows the window's appearance and gives the kit's own widgets our colors and text size.
pub fn sync(window: &mut Window, cx: &mut App) {
    Kit::sync_system_appearance(Some(window), cx);
    sync_kit(cx);
}

fn sync_kit(cx: &mut App) {
    if !cx.has_global::<Kit>() {
        return;
    }
    let t = theme(cx);
    let fonts = cx
        .try_global::<Active>()
        .map(|a| (a.font.clone(), a.mono.clone()));
    Kit::update(cx, |k| {
        if let Some((font, mono)) = fonts {
            (k.font_family, k.mono_font_family) = (font, mono);
        }
        k.font_size = text::BODY;
        k.background = t.ground;
        k.foreground = t.fg;
        k.muted_foreground = t.muted;
        k.border = t.line;
        k.input = t.line;
        k.ring = t.selected_ring;
        k.selection = t.text_selection;
        k.accent = t.hover;
        k.accent_foreground = t.fg;
        k.popover = t.popup;
        k.popover_foreground = t.fg;
        k.list_hover = t.hover;
        k.list_active = t.selected;
        k.primary = t.primary;
        k.primary_foreground = t.primary_fg;
        k.danger = t.error;
        k.warning = t.warning;
        k.success = t.success;
        let mut code = (*k.highlight_theme).clone();
        code.style.editor_background = Some(t.code);
        code.style.editor_gutter_background = Some(t.code);
        code.style.editor_active_line = Some(t.line_highlight);
        k.highlight_theme = Arc::new(code);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bundled_themes_are_complete_on_their_own() {
        for (name, v) in bundled() {
            for mode in ["light", "dark"] {
                let m = v[mode].as_object().unwrap();
                parse(m).unwrap_or_else(|e| panic!("{name} {mode}: {e}"));
            }
        }
        assert_eq!(bundled().len(), BUNDLED.len());
    }

    #[test]
    fn a_bad_token_says_which_and_why() {
        let v: Value = serde_json::from_str(r##"{"dark": {"fg": "red"}}"##).unwrap();
        let e = pair(&v, &bundled()).err().unwrap();
        assert_eq!(e, r#"dark.fg: "red" is not #rrggbb or #rrggbbaa"#);
        let v: Value = serde_json::from_str(r#"{"extends": "Nope"}"#).unwrap();
        assert!(pair(&v, &bundled()).is_err());
    }
}
