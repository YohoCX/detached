mod common;

use common::*;
use detached_app::ui::theme;
use gpui_kit::component::Theme as Kit;
use gpui_kit::{Hsla, TestAppContext, VisualTestContext, rgb};

const MINE: &str = "themes/mine.json";

fn mine(accent: &str) -> String {
    format!(
        r#"{{"name": "Mine", "light": {{"accent": "{accent}"}}, "dark": {{"accent": "{accent}"}}}}"#
    )
}

fn accent(cx: &mut VisualTestContext) -> Hsla {
    cx.update(|_, cx| theme::theme(cx).accent)
}

fn red() -> Hsla {
    rgb(0xff0000).into()
}

#[gpui_kit::test]
fn an_edited_theme_shows_and_a_broken_one_keeps_the_last(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, |sb| {
        let config = &sb.state.config_dir;
        std::fs::create_dir_all(config.join("themes")).unwrap();
        std::fs::write(config.join(MINE), mine("#ff0000")).unwrap();
        std::fs::write(config.join("ui.json"), r#"{"theme.name": "Mine"}"#).unwrap();
    });
    assert_eq!(accent(cx), red(), "only the accent is its own");
    let file = cx.sb.state.config_dir.join(MINE);
    std::fs::write(&file, mine("#00ff00")).unwrap();
    let green: Hsla = rgb(0x00ff00).into();
    wait_until(cx, |cx| accent(cx) == green);

    std::fs::write(&file, "{ not json").unwrap();
    wait_until(cx, |cx| {
        cx.store(|s, _| {
            s.message
                .as_ref()
                .is_some_and(|m| m.text.contains("Theme Mine not loaded"))
        })
    });
    assert_eq!(accent(cx), green);
    let error = cx.update(|_, cx| theme::error(cx)).unwrap();
    assert!(error.contains("themes/mine.json"), "{error}");
}

#[gpui_kit::test]
fn another_theme_is_chosen_and_kept_across_a_restart(cx: &mut TestAppContext) {
    let cx = &mut open_app(cx);
    let fleet = accent(cx);
    let fonts = |cx: &mut VisualTestContext| {
        cx.update(|_, cx| {
            let k = Kit::global(cx);
            (k.font_family.to_string(), k.mono_font_family.to_string())
        })
    };
    assert_eq!(fonts(cx), ("Inter".into(), "JetBrains Mono".into()));
    cx.update(|_, cx| theme::choose("Classic", cx));
    wait_until(cx, |cx| accent(cx) != fleet);
    assert_ne!(fonts(cx).0, "Inter", "Classic keeps the system's");
    let ui = std::fs::read_to_string(cx.sb.state.config_dir.join("ui.json")).unwrap();
    assert!(ui.contains(r#""theme.name": "Classic""#), "{ui}");
}

#[gpui_kit::test]
fn opening_the_themes_folder_seeds_a_copy_to_edit(_cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    theme::seed(dir.path()).unwrap();
    let text = std::fs::read_to_string(dir.path().join("my-theme.json")).unwrap();
    assert!(text.contains(r#""name": "My theme""#));
    std::fs::write(dir.path().join("my-theme.json"), "{}").unwrap();
    theme::seed(dir.path()).unwrap();
    assert_eq!(
        std::fs::read_to_string(dir.path().join("my-theme.json")).unwrap(),
        "{}"
    );
}
