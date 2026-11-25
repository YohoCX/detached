//! Colors from theme files, the bundled ones and the user's `themes/*.json`, light or dark as the
//! mode says. A file only needs the tokens it changes; the rest come from `extends`.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, SystemTime};

use gpui_kit::component::{Theme as Kit, ThemeMode};
use gpui_kit::{App, BoxShadow, Global, Hsla, Pixels, SharedString, Window, px, rgba};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::util::Status;
use kernel::activity::Activity;
use kernel::blocking;
use kernel::ui_state;
use kernel::workspace::Workspace;

pub const DEFAULT: &str = "Slate";
const BUNDLED: [&str; 4] = [
    include_str!("../../themes/slate.json"),
    include_str!("../../themes/fleet.json"),
    include_str!("../../themes/classic.json"),
    include_str!("../../themes/catppuccin.json"),
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
const MODE: &str = "theme.mode";
/// How often the themes folder is looked at for edits.
const POLL: Duration = Duration::from_millis(500);

macro_rules! tokens {
    ($($color:ident),* ; $($shadow:ident),* ; $($list:ident: $n:literal),* ; $($derived:ident),*) => {
        pub struct Theme {
            $(pub $color: Hsla,)*
            $(pub $shadow: Vec<BoxShadow>,)*
            $(pub $list: [Hsla; $n],)*
            $(pub $derived: Hsla,)*
        }

        fn parse(m: &Map<String, Value>) -> Result<Theme, String> {
            Ok(Theme {
                $($color: color(m, stringify!($color))?,)*
                $($shadow: shadows(m, stringify!($shadow))?,)*
                $($list: colors(m, stringify!($list))?,)*
                $($derived: Hsla::default(),)*
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
    ansi: 16, lanes: 8;
    // Not in theme files: `derive` makes them from the colors above.
    window_bg, panel_bg, header_bg, popover_bg, popover_selected, segment_bg, panel_edge, active_bg,
    tab_active, tab_unfocused, user_bg, code_block_bg
);

/// The gap between panes and around the window.
pub const GAP: Pixels = px(10.);

/// A tab's title is cut past this, so a long one leaves its close button in the zone.
pub const TAB_TITLE: Pixels = px(240.);

pub mod control {
    use gpui_kit::{Pixels, px};

    pub const HEIGHT: Pixels = px(28.);
    pub const GAP: Pixels = px(6.);
    pub const ICON: Pixels = px(14.);
    pub const TEXT_PAD: Pixels = px(14.);
}

pub mod segment {
    use gpui_kit::{Pixels, px};

    pub const HEIGHT: Pixels = px(24.);
    pub const INSET: Pixels = px(2.);
    pub const PAD: Pixels = px(10.);
    pub const GAP: Pixels = px(4.);
    pub const ICON: Pixels = px(14.);
    pub const TIP_RISE: Pixels = px(-40.);
}

pub mod rail {
    use gpui_kit::{Pixels, px};

    pub const WIDTH: Pixels = px(36.);
    pub const PAD: Pixels = px(8.);
    pub const GAP: Pixels = px(4.);
    pub const CONTROL: Pixels = super::control::HEIGHT;
    pub const ICON: Pixels = super::control::ICON;
}

pub mod chat {
    use gpui_kit::{Pixels, px};

    pub const INSET: Pixels = px(20.);
    pub const CONVERSATION: Pixels = px(820.);
    pub const PROSE: Pixels = px(760.);
    pub const INLINE: Pixels = px(640.);
    pub const WEEK: Pixels = px(720.);
    pub const MENU: Pixels = px(280.);
    pub const OPTION: Pixels = px(32.);
    pub const CONTROL: Pixels = px(28.);
    pub const PAD: Pixels = px(8.);
    pub const CONTROL_PAD: Pixels = px(8.);
    pub const GAP: Pixels = px(6.);
    pub const CHECK: Pixels = px(14.);
    pub const CHEVRON: Pixels = px(10.);
    pub const LINE: Pixels = px(1.);
}

pub mod radius {
    use gpui_kit::{Pixels, px};

    pub const PILL: Pixels = px(999.);
    pub const DIALOG: Pixels = px(20.);
    pub const PALETTE: Pixels = px(18.);
    pub const GLASS: Pixels = px(16.);
    pub const MENU: Pixels = px(12.);
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
    /// A Settings page's title.
    pub const PAGE: Pixels = px(18.);
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
    // Existing custom themes keep Fleet as their base.
    let base = v["extends"].as_str().unwrap_or("Fleet");
    let base = bases
        .get(base)
        .ok_or(format!("extends {base}, which is not a bundled theme"))?;
    let side = |mode: &str| {
        let mut m = base[mode].as_object().cloned().unwrap_or_default();
        if let Some(own) = v[mode].as_object() {
            m.extend(own.clone());
        }
        parse(&m)
            .map(|t| t.derive(mode == "dark"))
            .map_err(|e| format!("{mode}.{e}"))
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
            &format!("\"name\": \"My theme\",\n  \"extends\": \"{DEFAULT}\""),
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

/// Every theme that loads, by name, with its light and dark tokens.
pub fn previews(dir: &Path) -> Vec<(String, Theme, Theme)> {
    let (names, _) = find(dir, DEFAULT, None);
    let load = |n: String| find(dir, &n, None).1.ok().map(|l| (n, l.light, l.dark));
    names.into_iter().filter_map(load).collect()
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
    /// The surfaces the shell paints: the authored panel, the window and bands stepped away from it.
    fn derive(mut self, dark: bool) -> Self {
        let (black, white) = (gpui_kit::black(), gpui_kit::white());
        // `a` moved `w` of the way to opaque `b`.
        let mix = |a: Hsla, b: Hsla, w: f32| a.blend(b.opacity(w));
        let g = black.blend(self.ground);
        let p = g.blend(self.panel);
        let (f, a, u) = (p.blend(self.fg), p.blend(self.accent), g.blend(self.popup));
        self.panel_bg = p;
        if dark {
            self.window_bg = mix(g, black, 0.8);
            self.header_bg = mix(p, f, 0.10);
            self.popover_bg = mix(mix(self.header_bg, u, 0.25), f, 0.04);
            self.segment_bg = f.opacity(0.12);
        } else {
            self.window_bg = mix(g, f, 0.03);
            self.header_bg = mix(p, self.window_bg, 0.7);
            self.popover_bg = mix(u, white, 0.5);
            self.segment_bg = self.popover_bg;
        }
        let (edge, user, code) = if dark {
            (0.4, 0.15, 0.65)
        } else {
            (0.37, 0.09, 0.68)
        };
        self.panel_edge = mix(self.panel_bg.blend(self.panel_line), f, edge);
        self.active_bg = p.blend(self.selected);
        self.tab_active = self.header_bg.blend(self.selected);
        self.tab_unfocused = self.header_bg.blend(self.selected_inactive);
        // Dark popovers start lighter than panels, leaving less contrast for status inks.
        self.popover_selected = self.popover_bg.blend(Hsla {
            a: self.selected.a.min(if dark { 0.07 } else { 1. }),
            ..self.selected
        });
        self.user_bg = mix(self.panel_bg, a, user);
        self.code_block_bg = mix(self.panel_bg.blend(self.code), self.window_bg, code);
        self
    }

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

/// Whether the kit's theme last synced to dark.
pub fn is_dark(cx: &App) -> bool {
    cx.try_global::<Kit>().is_some_and(|k| k.is_dark())
}

/// The tokens for the appearance the kit's theme last synced to.
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

/// The family the window's text is set in.
pub fn sans(cx: &App) -> SharedString {
    Kit::global(cx).font_family.clone()
}

/// Light, dark, or the window's appearance.
#[derive(Clone, Copy, Default, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    System,
    Light,
    Dark,
}

pub fn mode(cx: &App) -> Mode {
    ui_state::get(MODE, cx).unwrap_or_default()
}

pub fn set_mode(mode: Mode, window: &mut Window, cx: &mut App) {
    ui_state::set(MODE, mode, cx);
    sync(window, cx);
}

/// Sets the kit's appearance from the mode and gives its widgets our colors and text size.
pub fn sync(window: &mut Window, cx: &mut App) {
    match mode(cx) {
        Mode::System => Kit::sync_system_appearance(Some(window), cx),
        Mode::Light => Kit::change(ThemeMode::Light, Some(window), cx),
        Mode::Dark => Kit::change(ThemeMode::Dark, Some(window), cx),
    }
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
        k.background = t.window_bg;
        k.foreground = t.fg;
        k.muted_foreground = t.muted;
        k.border = t.line;
        k.input = t.line;
        k.ring = t.selected_ring;
        k.selection = t.text_selection;
        k.accent = t.hover;
        k.accent_foreground = t.fg;
        k.popover = t.popover_bg;
        k.popover_foreground = t.fg;
        k.list_hover = t.hover;
        k.list_active = t.active_bg;
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

    #[test]
    fn a_theme_extends_any_bundled_one() {
        for name in bundled().into_keys() {
            let v = serde_json::json!({"extends": name, "dark": {"accent": "#ff0000"}});
            let (light, dark) = pair(&v, &bundled()).unwrap();
            let (base, _) = pair(&bundled()[&name], &bundled()).unwrap();
            assert_eq!(light.accent, base.accent, "{name}");
            assert_eq!(dark.accent, hex("#ff0000").unwrap(), "{name}");
        }
    }

    /// WCAG's contrast ratio of two opaque colors.
    fn contrast(a: Hsla, b: Hsla) -> f32 {
        let lum = |c: Hsla| {
            let c = gpui_kit::Rgba::from(c);
            let f = |v: f32| match v <= 0.04045 {
                true => v / 12.92,
                false => ((v + 0.055) / 1.055).powf(2.4),
            };
            0.2126 * f(c.r) + 0.7152 * f(c.g) + 0.0722 * f(c.b)
        };
        let (a, b) = (lum(a), lum(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    fn hex_of(c: Hsla) -> String {
        let c = gpui_kit::Rgba::from(c);
        let b = |v: f32| (v * 255.).round() as u8;
        format!("#{:02x}{:02x}{:02x}", b(c.r), b(c.g), b(c.b))
    }

    #[test]
    fn the_surfaces_step_apart_from_the_authored_colors() {
        let (light, dark) = pair(&bundled()[DEFAULT], &bundled()).unwrap();
        let surfaces = |t: &Theme| {
            [
                t.window_bg,
                t.panel_bg,
                t.header_bg,
                t.popover_bg,
                t.popover_selected,
                t.panel_edge,
                t.active_bg,
                t.user_bg,
                t.code_block_bg,
            ]
            .map(hex_of)
        };
        assert_eq!(
            surfaces(&light),
            [
                "#d9dde4", "#ffffff", "#e4e7ec", "#ffffff", "#dbe6f3", "#7f8388", "#dbe6f3",
                "#e8eff8", "#e5e8ed",
            ]
        );
        assert_eq!(
            surfaces(&dark),
            [
                "#070809", "#161a1f", "#2b2e33", "#31353a", "#373e48", "#7d8187", "#2e3d50",
                "#263241", "#0c0e11",
            ]
        );
    }

    #[test]
    fn slate_focused_tab_outline_contrasts_with_the_tab_and_strip() {
        let (light, dark) = pair(&bundled()[DEFAULT], &bundled()).unwrap();
        for (mode, t) in [("light", light), ("dark", dark)] {
            for bg in [t.tab_active, t.panel_bg] {
                let ratio = contrast(t.selected_ring, bg);
                assert!(ratio >= 3., "{mode} focused tab outline is {ratio:.2}:1");
            }
        }
    }

    #[test]
    fn text_keeps_4_5_to_1_on_every_ground_it_sits_on() {
        let black: Hsla = rgba(0x000000ff).into();
        for (name, v) in bundled() {
            let (light, dark) = pair(&v, &bundled()).unwrap();
            for (mode, t) in [("light", light), ("dark", dark)] {
                let ground = black.blend(t.ground);
                let pb = t.panel_bg;
                let text_grounds = [
                    ("window_bg", t.window_bg),
                    ("ground", ground),
                    ("segment track", t.header_bg.blend(t.hover)),
                    ("user_bg", t.user_bg),
                    ("code_block_bg", t.code_block_bg),
                ];
                let ink_grounds = [
                    ("popup", ground.blend(t.popup)),
                    ("popover_bg", t.popover_bg),
                    ("popover_selected", t.popover_selected),
                    ("menu hover", t.popover_bg.blend(t.hover)),
                    ("panel_bg", pb),
                    ("header_bg", t.header_bg),
                    ("code", pb.blend(t.code)),
                    ("card", pb.blend(t.hover)),
                    ("input", pb.blend(t.input)),
                    ("active_bg", t.active_bg),
                    ("selected_inactive", pb.blend(t.selected_inactive)),
                ];
                let mut pairs = vec![
                    ("primary_fg", t.primary_fg, "primary", t.primary),
                    ("accent_fg", t.accent_fg, "accent", t.accent),
                    ("fg", t.fg, "active tab", t.tab_active),
                    ("fg", t.fg, "unfocused or hovered tab", t.tab_unfocused),
                    (
                        "fg",
                        t.fg,
                        "segment",
                        t.header_bg.blend(t.hover).blend(t.segment_bg),
                    ),
                ];
                for (g, bg) in text_grounds.into_iter().chain(ink_grounds) {
                    pairs.push(("fg", t.fg, g, bg));
                    pairs.push(("muted", t.muted, g, bg));
                }
                let colored = [
                    ("accent", t.accent),
                    ("hit", t.hit),
                    ("error", t.error),
                    ("warning", t.warning),
                    ("success", t.success),
                    ("added", t.added),
                    ("modified", t.modified),
                    ("deleted", t.deleted),
                    ("conflict", t.conflict),
                    ("untracked", t.untracked),
                ];
                for (k, c) in colored {
                    for (g, bg) in ink_grounds {
                        pairs.push((k, c, g, bg));
                    }
                }
                for (k, c, g, bg) in pairs {
                    let r = contrast(c, bg);
                    assert!(r >= 4.5, "{name} {mode}: {k} on {g} is {r:.2}:1");
                }
            }
        }
    }
}

pub mod graph {
    use gpui_kit::{Pixels, px};
    pub const HISTORY_ROW: Pixels = px(40.);
    pub const HISTORY_RAIL: Pixels = px(20.);
    pub const RAIL: Pixels = px(2.);
    pub const DOT: Pixels = px(8.);
    pub const ROW: Pixels = px(64.);
    pub const FLOW_COLUMN: f32 = 240.;
    pub const FLOW_GUTTER: f32 = 64.;
    pub const FLOW_HEADER: f32 = 40.;
    pub const FLOW_CARD: f32 = 56.;
    pub const TIMELINE_NAME: f32 = 140.;
    pub const TIMELINE_LINE: f32 = 22.;
    pub const TIMELINE_MARK: f32 = 14.;
    pub const TIMELINE_TICK: f32 = 64.;
    pub const CHART: Pixels = px(150.);
}
