mod common;

use common::*;
use detached_app::settings;
use detached_core::skills::{BLOCK, Config};
use detached_core::store::{Kind, RepoStore};
use gpui_kit::TestAppContext;
use std::path::Path;

fn skill(dir: &Path, text: &str) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join("SKILL.md"), text).unwrap();
}

fn label(cx: &mut TestApp, id: &'static str) -> String {
    cx.find(id).unwrap().label().unwrap().to_string()
}

#[gpui_kit::test]
fn the_skills_page_lists_each_skill_with_its_source_and_stops_reading_agents(
    cx: &mut TestAppContext,
) {
    let cx = &mut open_app_with(cx, |sb| {
        let claude = sb.path("claude").display().to_string();
        let mut env = sb.state.env.write().unwrap();
        env.vars.push(("CLAUDE_CONFIG_DIR".into(), claude));
        drop(env);
        let r = repo(sb, "main");
        let local = [("SKILL.md".into(), b"local".to_vec())].into();
        let store = RepoStore::open(&sb.state, &r).unwrap();
        store
            .write(&sb.state, Kind::Skills, "lint", &local)
            .unwrap();
        skill(&r.join(".detached/skills/team"), "team");
        skill(
            &sb.path("home/.agents/skills/lint"),
            "---\ndescription: Lints\n---\n",
        );
    });
    let main = cx.sb.path("repos/main").display().to_string();
    cx.workspace(|w, cx| w.set_current(Some(main.clone()), cx));
    cx.update(|_, cx| settings::open(cx, "skills"));
    wait_until(cx, |cx| cx.find("skills.Agents.lint").is_some());
    assert_eq!(label(cx, "skills.Local.lint"), "lint · local");
    assert_eq!(
        label(cx, "skills.Shared.team"),
        "team · shared · needs acceptance on the Sharing page"
    );
    assert_eq!(
        label(cx, "skills.Agents.lint"),
        "lint · ~/.agents · overridden by local"
    );
    let agents = cx.sb.path("home/.agents/skills").display().to_string();
    assert_eq!(
        label(cx, "skills.folder.Agents"),
        format!("~/.agents: {agents}")
    );
    assert_eq!(label(cx, "skills.folder.Agents.path"), agents);
    assert_eq!(cx.find("skills.agents").unwrap().checked(), Some(true));
    cx.click("skills.agents");
    wait_until(cx, |cx| cx.find("skills.Agents.lint").is_none());
    assert_eq!(
        label(cx, "skills.folder.Agents"),
        format!("~/.agents: {agents} · off")
    );
    assert_eq!(cx.find("skills.agents").unwrap().checked(), Some(false));
    let config = cx.update(|_, cx| kernel::core(cx).get_settings().block::<Config>(BLOCK));
    assert_eq!(config, Some(Config { agents: false }));
}
