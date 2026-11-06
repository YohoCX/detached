//! Default chords from docs/KEYMAP.md, `keymap.json` overrides by action id, Double Shift.
//! Off macOS, Cmd is Ctrl. Chords are bound in the shell's context, so a kit text field or editor
//! that binds the same chord keeps it while focused.

use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use detached_core::{api, state::AppState};
use gpui_kit::{App, Global, KeyBinding, KeyBindingContextPredicate, Keystroke, Modifiers, Task};

use crate::actions::{self, Run};
use crate::blocking;

const MAC: bool = cfg!(target_os = "macos");

/// The shell's key context; it holds focus whenever nothing inside it does.
pub const CONTEXT: &str = "Detached";

/// Around the viewer's read-only editor, where the app's chords beat the editor's own.
pub const VIEWER: &str = "Viewer";

/// Append-only, in the order bindings are tried.
pub const DEFAULTS: &[(&str, &str)] = &[
    ("search.everywhere", "Double Shift"),
    ("file.goto", "Cmd+Shift+O"),
    ("find.inFiles", "Cmd+Shift+F"),
    ("action.find", "Cmd+Shift+A"),
    ("file.recent", "Cmd+E"),
    ("editor.gotoLine", "Cmd+L"),
    ("nav.back", "Cmd+["),
    ("nav.forward", "Cmd+]"),
    ("panel.project", "Cmd+1"),
    ("vcs.update", "Cmd+T"),
    ("vcs.popup", "Ctrl+V"),
    ("diff.show", "Cmd+D"),
    ("diff.next", "F7"),
    ("diff.prev", "Shift+F7"),
    ("app.settings", "Cmd+,"),
    ("panel.log", "Cmd+9"),
    ("panel.terminal", "Alt+F12"),
];

/// Chords kept free for actions that do not exist yet.
pub const RESERVED: &[(&str, &str)] = &[
    ("panel.commit", "Cmd+0"),
    ("vcs.commit", "Cmd+K"),
    ("vcs.push", "Cmd+Shift+K"),
    ("vcs.rollback", "Cmd+Alt+Z"),
];

/// Ctrl+V pastes off macOS, so the VCS popup is Alt+` there.
pub fn defaults(mac: bool) -> impl Iterator<Item = (&'static str, &'static str)> {
    DEFAULTS.iter().map(move |&(id, s)| match id {
        "vcs.popup" if !mac => (id, "Alt+`"),
        _ => (id, s),
    })
}

#[derive(Clone, Debug, PartialEq)]
pub enum Chord {
    DoubleShift,
    Keys(Keys),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Keys {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    /// Cmd on macOS, the Windows or Super key elsewhere.
    pub meta: bool,
    /// GPUI's key name: `o`, `[`, `f7`, `escape`.
    pub key: String,
    /// As written, a single character upper-cased.
    pub label: String,
}

impl Keys {
    /// GPUI's keystroke syntax, e.g. `ctrl-shift-o`.
    pub fn keystroke(&self) -> String {
        let mods = [
            (self.ctrl, "ctrl-"),
            (self.alt, "alt-"),
            (self.shift, "shift-"),
            (self.meta, "cmd-"),
        ];
        let mut s: String = mods.iter().filter(|m| m.0).map(|m| m.1).collect();
        s.push_str(&self.key);
        s
    }
}

fn key_name(k: &str) -> Option<String> {
    let k = k.to_lowercase();
    let single =
        k.len() == 1 && (k.as_bytes()[0].is_ascii_alphanumeric() || "[],./;'-=`\\".contains(&k));
    let named =
        "enter escape tab space backspace delete up down left right home end pageup pagedown";
    let f = (1..=24).any(|n| k == format!("f{n}"));
    match k.as_str() {
        "esc" => Some("escape".into()),
        _ if single || f || named.split(' ').any(|n| n == k) => Some(k),
        _ => None,
    }
}

/// "Cmd+Shift+O", "Ctrl+V", "F7", "Cmd+[", "Double Shift"; None when invalid or empty.
pub fn parse(s: &str, mac: bool) -> Option<Chord> {
    let s = s.trim();
    if let [d, sh] = s.split_whitespace().collect::<Vec<_>>()[..]
        && d.eq_ignore_ascii_case("double")
        && sh.eq_ignore_ascii_case("shift")
    {
        return Some(Chord::DoubleShift);
    }
    let mut parts: Vec<&str> = s.split('+').map(str::trim).collect();
    let label = parts.pop()?;
    let mut k = Keys {
        key: key_name(label)?,
        label: if label.chars().count() == 1 {
            label.to_uppercase()
        } else {
            label.into()
        },
        ..Keys::default()
    };
    for p in parts {
        let m = match p.to_lowercase().as_str() {
            "cmd" | "command" if mac => &mut k.meta,
            "cmd" | "command" | "ctrl" | "control" => &mut k.ctrl,
            "meta" => &mut k.meta,
            "alt" | "opt" | "option" => &mut k.alt,
            "shift" => &mut k.shift,
            _ => return None,
        };
        *m = true;
    }
    Some(Chord::Keys(k))
}

/// macOS symbols in the order ⌃⌥⇧⌘; elsewhere `Ctrl+Alt+Shift+Meta+Key`.
pub fn format(c: &Chord, mac: bool) -> String {
    let k = match c {
        Chord::DoubleShift => return if mac { "⇧⇧" } else { "Shift Shift" }.into(),
        Chord::Keys(k) => k,
    };
    let names = if mac {
        ["⌃", "⌥", "⇧", "⌘"]
    } else {
        ["Ctrl", "Alt", "Shift", "Meta"]
    };
    let on = [k.ctrl, k.alt, k.shift, k.meta];
    let mut parts: Vec<&str> = names
        .iter()
        .zip(on)
        .filter(|(_, on)| *on)
        .map(|(n, _)| *n)
        .collect();
    parts.push(&k.label);
    parts.join(if mac { "" } else { "+" })
}

/// Defaults with overrides applied, in default order, then overrides of other actions; an override
/// that does not parse, an empty one included, unbinds its action. Unknown ids are dropped.
pub fn bindings(overrides: &BTreeMap<String, String>, mac: bool) -> Vec<(&'static str, Chord)> {
    let mut out: Vec<(&'static str, Option<Chord>)> =
        defaults(mac).map(|(id, s)| (id, parse(s, mac))).collect();
    for (id, s) in overrides {
        let Some(a) = actions::find(id) else { continue };
        let c = parse(s, mac);
        match out.iter_mut().find(|(i, _)| *i == a.id) {
            Some(slot) => slot.1 = c,
            None => out.push((a.id, c)),
        }
    }
    out.into_iter()
        .filter_map(|(id, c)| Some((id, c?)))
        .collect()
}

const DOUBLE: Duration = Duration::from_millis(300);

/// Two lone Shift taps within 300 ms; any other key or a mouse press between them resets it. GPUI
/// reports a lone modifier as a keystroke of its own on release, even after a Shift-click.
#[derive(Default)]
pub struct DoubleShift {
    last: Option<Instant>,
}

impl DoubleShift {
    pub fn key(&mut self, k: &Keystroke, now: Instant) -> bool {
        let lone = k.key == "shift" && k.modifiers == Modifiers::none();
        let fired = lone
            && self
                .last
                .is_some_and(|t| now.saturating_duration_since(t) <= DOUBLE);
        self.last = (lone && !fired).then_some(now);
        fired
    }

    pub fn reset(cx: &mut App) {
        cx.default_global::<DoubleShift>().last = None;
    }
}

impl Global for DoubleShift {}

struct Bound(Vec<(&'static str, Chord)>);

impl Global for Bound {}

/// The action bound to `k`, if any.
pub fn action_of(k: &Keystroke, cx: &App) -> Option<&'static str> {
    let bound = cx.try_global::<Bound>()?;
    bound.0.iter().find_map(|(id, c)| {
        let Chord::Keys(c) = c else { return None };
        let m = &k.modifiers;
        let same = (c.ctrl, c.alt, c.shift, c.meta) == (m.control, m.alt, m.shift, m.platform);
        (same && c.key == k.key).then_some(*id)
    })
}

/// The chord shown next to an action's name; empty when unbound.
pub fn chord_for(id: &str, cx: &App) -> String {
    let bound = cx
        .try_global::<Bound>()
        .and_then(|b| b.0.iter().find(|(i, _)| *i == id));
    bound.map_or(String::new(), |(_, c)| format(c, MAC))
}

/// Reads `keymap.json` on the pool, then binds; menus showing chords are built once it resolves.
pub fn load(core: Arc<AppState>, cx: &mut App) -> Task<()> {
    let overrides = blocking::run(cx, move || api::keymap_overrides(&core));
    cx.spawn(async move |cx| {
        let overrides = overrides.await;
        cx.update(|cx| bind(&overrides, cx));
    })
}

/// Binds the defaults with `overrides`; once per app.
pub fn bind(overrides: &BTreeMap<String, String>, cx: &mut App) {
    let bound = bindings(overrides, MAC);
    // Bound after the kit's, so in the viewer they beat its editor's Outdent, Indent and Replace.
    let viewer = format!("{VIEWER} > Input");
    let contexts =
        [CONTEXT, &viewer].map(|c| KeyBindingContextPredicate::parse(c).ok().map(Rc::new));
    // GPUI tries the latest binding of a chord first; the first one listed has to win.
    let mut keys = vec![];
    for (id, c) in bound.iter().rev() {
        let Chord::Keys(k) = c else { continue };
        for context in &contexts {
            let run = Box::new(Run(id));
            keys.extend(
                KeyBinding::load(&k.keystroke(), run, context.clone(), false, None, cx).ok(),
            );
        }
    }
    cx.bind_keys(keys);
    if let Some(&(id, _)) = bound.iter().find(|(_, c)| *c == Chord::DoubleShift) {
        cx.observe_keystrokes(move |e, window, cx| {
            let now = cx.background_executor().now();
            if cx.default_global::<DoubleShift>().key(&e.keystroke, now) {
                actions::dispatch(window, cx, id);
            }
        })
        .detach();
    }
    cx.set_global(Bound(bound));
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use gpui_kit::component::input::{Editor, EditorState};
    use gpui_kit::{
        AppContext, Context, Entity, FocusHandle, InteractiveElement, IntoElement,
        KeybindingKeystroke, OperatingSystem, ParentElement, Render, TestAppContext,
        VisualTestContext, Window, div, prelude::FluentBuilder,
    };

    use super::*;

    fn keys(s: &str) -> Keys {
        match parse(s, true) {
            Some(Chord::Keys(k)) => k,
            c => panic!("{s}: {c:?}"),
        }
    }

    #[test]
    fn parses_modifiers_and_keys() {
        let o = Keys {
            meta: true,
            shift: true,
            key: "o".into(),
            label: "O".into(),
            ..Keys::default()
        };
        assert_eq!(parse("Cmd+Shift+O", true), Some(Chord::Keys(o)));
        assert_eq!(keys("Ctrl+V").keystroke(), "ctrl-v");
        assert_eq!(keys("F7").keystroke(), "f7");
        assert_eq!(keys("Shift+F7").keystroke(), "shift-f7");
        assert_eq!(keys("Cmd+[").keystroke(), "cmd-[");
        assert_eq!(keys("Cmd+]").keystroke(), "cmd-]");
        assert_eq!(keys("cmd+alt+z").keystroke(), "alt-cmd-z");
        assert_eq!(keys("Cmd+1").keystroke(), "cmd-1");
        assert_eq!(keys("Cmd+Esc").keystroke(), "cmd-escape");
        assert_eq!(parse("Double Shift", true), Some(Chord::DoubleShift));
        assert_eq!(parse(" double  SHIFT ", true), Some(Chord::DoubleShift));
    }

    #[test]
    fn rejects_invalid_chords() {
        for s in [
            "",
            "Cmd+",
            "Hyper+X",
            "Cmd+Shift",
            "F25",
            "Cmd+Foo",
            "Cmd+é",
        ] {
            assert_eq!(parse(s, true), None, "{s}");
        }
    }

    #[test]
    fn formats_with_macos_symbols() {
        let f = |s| format(&parse(s, true).unwrap(), true);
        assert_eq!(f("Cmd+Shift+O"), "⇧⌘O");
        assert_eq!(f("Ctrl+V"), "⌃V");
        assert_eq!(f("Cmd+["), "⌘[");
        assert_eq!(f("Double Shift"), "⇧⇧");
    }

    #[test]
    fn reads_cmd_as_ctrl_on_windows_and_linux() {
        let c = parse("Cmd+Shift+O", false).unwrap();
        let Chord::Keys(k) = &c else { panic!() };
        assert!(k.ctrl && !k.meta && k.shift);
        assert_eq!(format(&c, false), "Ctrl+Shift+O");
        assert_eq!(
            format(&parse("Meta+Alt+[", false).unwrap(), false),
            "Alt+Meta+["
        );
        assert_eq!(format(&Chord::DoubleShift, false), "Shift Shift");
    }

    /// GPUI matches the typed key, which on a non-Latin layout is its Latin equivalent.
    fn matches(chord: &str, key: &str, key_char: Option<&str>, m: Modifiers) -> bool {
        let target =
            Keystroke::parse_with_os(&keys(chord).keystroke(), OperatingSystem::Mac).unwrap();
        let typed = Keystroke {
            modifiers: m,
            key: key.into(),
            key_char: key_char.map(Into::into),
        };
        typed.should_match(&KeybindingKeystroke::from_keystroke(target))
    }

    fn cmd() -> Modifiers {
        Modifiers::command()
    }

    #[test]
    fn matches_other_layouts_by_their_latin_key() {
        assert!(matches("Cmd+[", "[", None, cmd()));
        assert!(matches("Cmd+[", "[", Some("х"), cmd()));
        assert!(matches(
            "Cmd+Shift+O",
            "o",
            Some("Щ"),
            Modifiers {
                shift: true,
                ..cmd()
            }
        ));
        assert!(!matches("Cmd+E", "r", None, cmd()));
    }

    #[test]
    fn requires_exact_modifiers() {
        let shift = Modifiers::shift();
        assert!(matches("F7", "f7", None, Modifiers::none()));
        assert!(!matches("F7", "f7", None, shift));
        assert!(matches("Shift+F7", "f7", None, shift));
        assert!(!matches("Cmd+Shift+O", "o", None, cmd()));
        assert!(!matches("Ctrl+V", "v", None, cmd()));
    }

    #[test]
    fn binds_every_default_to_an_action_and_keeps_reserved_chords_free() {
        for mac in [true, false] {
            let b = bindings(&BTreeMap::new(), mac);
            for (id, _) in defaults(mac) {
                assert!(actions::find(id).is_some(), "{id}");
                assert!(b.iter().any(|(i, _)| *i == id), "{id}");
            }
            for (id, s) in RESERVED {
                assert!(actions::find(id).is_none(), "{id}");
                assert!(defaults(mac).all(|(_, d)| d != *s), "{s}");
            }
        }
        let popup = |mac| {
            bindings(&BTreeMap::new(), mac)
                .into_iter()
                .find(|(i, _)| *i == "vcs.popup")
        };
        assert_eq!(format(&popup(true).unwrap().1, true), "⌃V");
        assert_eq!(format(&popup(false).unwrap().1, false), "Alt+`");
    }

    #[test]
    fn keymap_md_lists_every_action_with_its_default_chord() {
        let md = include_str!("../../../docs/KEYMAP.md");
        for a in actions::ACTIONS {
            let id = format!("`{}`", a.id);
            let row = md.lines().find(|l| l.starts_with("| `") && l.contains(&id));
            let row = row.unwrap_or_else(|| panic!("{} is not in docs/KEYMAP.md", a.id));
            if let Some((_, chord)) = DEFAULTS.iter().find(|(i, _)| *i == a.id) {
                assert!(row.contains(chord), "{}: {chord}", a.id);
            }
        }
    }

    #[test]
    fn applies_overrides_and_an_empty_or_invalid_one_unbinds() {
        let o: BTreeMap<String, String> = [
            ("file.goto", "Cmd+P"),
            ("nav.back", ""),
            ("diff.next", "nonsense"),
            ("vcs.fetch", "Cmd+Shift+T"),
            ("no.such", "Cmd+J"),
        ]
        .map(|(a, b)| (a.into(), b.into()))
        .into();
        let b = bindings(&o, true);
        let get = |id| b.iter().find(|(i, _)| *i == id).map(|(_, c)| c.clone());
        assert_eq!(get("file.goto"), parse("Cmd+P", true));
        assert_eq!(get("nav.back"), None);
        assert_eq!(get("diff.next"), None);
        assert_eq!(get("nav.forward"), parse("Cmd+]", true));
        assert_eq!(get("vcs.fetch"), parse("Cmd+Shift+T", true));
        assert_eq!(b.last().unwrap().0, "vcs.fetch");
        assert!(b.iter().all(|(i, _)| actions::find(i).is_some()));
    }

    fn shift() -> Keystroke {
        Keystroke {
            key: "shift".into(),
            ..Keystroke::default()
        }
    }

    fn at(ms: u64, t: Instant) -> Instant {
        t + Duration::from_millis(ms)
    }

    #[test]
    fn double_shift_fires_on_two_lone_shifts_within_300_ms() {
        let (mut d, t) = (DoubleShift::default(), Instant::now());
        assert!(!d.key(&shift(), at(1000, t)));
        assert!(d.key(&shift(), at(1250, t)));
    }

    #[test]
    fn double_shift_does_not_fire_when_too_slow() {
        let (mut d, t) = (DoubleShift::default(), Instant::now());
        d.key(&shift(), at(1000, t));
        assert!(!d.key(&shift(), at(1301, t)));
        assert!(d.key(&shift(), at(1500, t)));
    }

    #[test]
    fn double_shift_is_interrupted_by_another_key_or_a_modifier_chord() {
        let (mut d, t) = (DoubleShift::default(), Instant::now());
        d.key(&shift(), at(1000, t));
        let a = Keystroke::parse("shift-a").unwrap();
        d.key(&a, at(1050, t));
        assert!(!d.key(&shift(), at(1100, t)));
        let chord = Keystroke {
            modifiers: Modifiers::command(),
            ..shift()
        };
        assert!(!d.key(&chord, at(1150, t)));
        assert!(!d.key(&shift(), at(1200, t)));
    }

    /// A held Shift is reported once, on release, so auto-repeat never reaches it.
    #[test]
    fn double_shift_needs_a_fresh_pair_after_firing() {
        let (mut d, t) = (DoubleShift::default(), Instant::now());
        d.key(&shift(), at(1000, t));
        assert!(d.key(&shift(), at(1150, t)));
        assert!(!d.key(&shift(), at(1200, t)));
        assert!(d.key(&shift(), at(1250, t)));
    }

    /// Every handler records its id, as the components that own them would run.
    fn record(
        cx: &mut App,
        ids: &[&'static str],
    ) -> (Rc<RefCell<Vec<&'static str>>>, Vec<actions::Registration>) {
        let ran = Rc::new(RefCell::new(vec![]));
        let regs = ids
            .iter()
            .map(|&id| {
                let ran = ran.clone();
                let push = move |_: &mut _, _: &mut _| ran.borrow_mut().push(id);
                actions::register(cx, id, "Title", actions::Group::View, push)
            })
            .collect();
        (ran, regs)
    }

    const CMD: &str = if MAC { "cmd" } else { "ctrl" };

    /// The shell's key context, focused, around the kit's editor, in the viewer's when `viewer`.
    struct Root {
        focus: FocusHandle,
        editor: Entity<EditorState>,
        viewer: bool,
    }

    impl Render for Root {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let editor = div()
                .when(self.viewer, |d| d.key_context(VIEWER))
                .child(Editor::new(&self.editor));
            div()
                .key_context(CONTEXT)
                .track_focus(&self.focus)
                .child(editor)
        }
    }

    fn root<'a>(
        cx: &'a mut TestAppContext,
        overrides: &BTreeMap<String, String>,
    ) -> (Entity<Root>, &'a mut VisualTestContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            actions::init(cx);
            bind(overrides, cx);
        });
        let (root, cx) = cx.add_window_view(|window, cx| {
            let focus = cx.focus_handle();
            window.focus(&focus, cx);
            let editor = cx.new(|cx| EditorState::new(window, cx));
            Root {
                focus,
                editor,
                viewer: false,
            }
        });
        cx.run_until_parked();
        (root, cx)
    }

    #[gpui_kit::test]
    fn dispatches_chords_and_double_shift(cx: &mut TestAppContext) {
        let (_root, cx) = root(cx, &BTreeMap::new());
        let ids = ["diff.prev", "editor.gotoLine", "search.everywhere"];
        let (ran, _regs) = cx.update(|_, cx| record(cx, &ids));
        cx.simulate_keystrokes(&format!("shift-f7 {CMD}-l x"));
        for _ in 0..2 {
            cx.simulate_modifiers_change(Modifiers::shift());
            cx.simulate_modifiers_change(Modifiers::none());
        }
        cx.run_until_parked();
        assert_eq!(*ran.borrow(), ids);
        cx.update(|_, cx| {
            assert_eq!(
                chord_for("diff.prev", cx),
                format(&parse("Shift+F7", MAC).unwrap(), MAC)
            )
        });
    }

    #[gpui_kit::test]
    fn uses_overrides(cx: &mut TestAppContext) {
        let o = [
            ("file.goto".to_string(), "Cmd+P".to_string()),
            ("nav.back".into(), "Cmd+Nope".into()),
        ];
        let (_root, cx) = root(cx, &o.into());
        let (ran, _regs) = cx.update(|_, cx| record(cx, &["file.goto", "nav.back"]));
        cx.simulate_keystrokes(&format!("{CMD}-shift-o {CMD}-p {CMD}-["));
        cx.run_until_parked();
        assert_eq!(*ran.borrow(), ["file.goto"]);
        cx.update(|_, cx| assert_eq!(chord_for("nav.back", cx), ""));
    }

    #[gpui_kit::test]
    fn the_focused_editor_keeps_its_own_chords_and_the_others_reach_the_app(
        cx: &mut TestAppContext,
    ) {
        let (root, cx) = root(cx, &BTreeMap::new());
        let (ran, _regs) = cx.update(|_, cx| record(cx, &["nav.back", "editor.gotoLine"]));
        cx.simulate_keystrokes(&format!("{CMD}-["));
        cx.run_until_parked();
        assert_eq!(
            *ran.borrow(),
            ["nav.back"],
            "outside the editor the chord is the app's"
        );

        let editor = cx.update(|window, cx| {
            let editor = root.read(cx).editor.clone();
            editor.update(cx, |e, cx| {
                e.set_value("    a", window, cx);
                e.focus(window, cx);
            });
            editor
        });
        cx.run_until_parked();
        cx.simulate_keystrokes(&format!("{CMD}-[ {CMD}-l"));
        cx.run_until_parked();
        assert_eq!(*ran.borrow(), ["nav.back", "editor.gotoLine"]);
        cx.update(|_, cx| assert_eq!(editor.read(cx).value(), "  a", "the editor outdented"));
    }

    #[gpui_kit::test]
    fn inside_the_viewer_the_apps_chords_beat_the_editors(cx: &mut TestAppContext) {
        let (root, cx) = root(cx, &BTreeMap::new());
        let (ran, _regs) = cx.update(|_, cx| record(cx, &["nav.back", "find.inFiles"]));
        let editor = cx.update(|window, cx| {
            root.update(cx, |r, _| r.viewer = true);
            let editor = root.read(cx).editor.clone();
            editor.update(cx, |e, cx| {
                e.set_value("    a", window, cx);
                e.focus(window, cx);
            });
            editor
        });
        cx.run_until_parked();
        cx.simulate_keystrokes(&format!("{CMD}-[ {CMD}-shift-f"));
        cx.run_until_parked();
        assert_eq!(*ran.borrow(), ["nav.back", "find.inFiles"]);
        cx.update(|_, cx| assert_eq!(editor.read(cx).value(), "    a"));
    }
}
