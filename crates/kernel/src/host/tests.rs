use std::sync::Arc;

use detached_core::api;
use detached_core::git::testutil::Sandbox;
use detached_core::types::{AccountInput, Provider};
use detached_forge_fake::{Forge, Kind, Mr, MrState, Project, Seed};
use gpui::TestAppContext;
use serde_json::json;
use tempfile::TempDir;

use super::*;
use crate::wait_until;

/// Repos `a`, on the forge fake with MRs 1 and 2, and `b`, with no remote, every status loaded.
fn boot(cx: &mut TestAppContext) -> (TempDir, Forge, String, String) {
    let mut sb = Sandbox::new();
    let claude = sb.path("claude").display().to_string();
    let env = sb.state.env.get_mut().unwrap();
    env.vars.push(("CLAUDE_CONFIG_DIR".into(), claude));
    let forge = Forge::start().unwrap();
    for name in ["a", "b"] {
        let repo = sb.path(&format!("repos/{name}"));
        std::fs::create_dir_all(&repo).unwrap();
        sb.git(&repo, &["init", "-q"]);
        sb.commit(&repo, "f", "x\n", "initial");
    }
    let a = sb.path("repos/a");
    sb.git(
        &a,
        &[
            "remote",
            "add",
            "origin",
            &format!("{}/group/app.git", forge.gitlab()),
        ],
    );
    let sha = sb.git(&a, &["rev-parse", "HEAD"]);
    let mr = |number, title: &str| Mr {
        number,
        title: title.into(),
        description: String::new(),
        author: "tester".into(),
        state: MrState::Open,
        draft: false,
        source_branch: format!("feature-{number}"),
        target_branch: "main".into(),
        base_sha: sha.clone(),
        head_sha: sha.clone(),
    };
    forge.reset(Seed {
        user: "tester".into(),
        projects: vec![Project {
            forge: Kind::Gitlab,
            path: "group/app".into(),
            default_branch: "main".into(),
            mrs: vec![mr(1, "One"), mr(2, "Two")],
            pipelines: vec![],
        }],
    });
    let account = AccountInput {
        id: None,
        provider: Provider::GitlabSelfHosted,
        base_url: forge.gitlab(),
        username: "tester".into(),
        label: "GitLab".into(),
        token: Some("gitlab-token".into()),
    };
    api::save_account(&sb.state, account).unwrap();
    let Sandbox { dir, state } = sb;
    let folder = dir.path().join("repos").display().to_string();
    state.add_folder("Default".into(), folder).unwrap();
    cx.update(|cx| crate::init(Arc::new(state), cx));
    wait_until(cx, |cx| {
        let repos = Workspace::get(cx).repos();
        repos.len() == 2
            && repos
                .iter()
                .all(|r| Repo::get(cx, &r.path).unwrap().status.is_some())
    });
    let path = |name: &str| {
        cx.read(|cx| {
            let repos = Workspace::get(cx).repos();
            repos.iter().find(|r| r.name == name).unwrap().path.clone()
        })
    };
    let (a, b) = (path("a"), path("b"));
    (dir, forge, a, b)
}

fn ask(cx: &mut TestAppContext, who: &Identity, q: &str) -> Value {
    let task = cx.update(|cx| query(cx, who, q));
    cx.executor().allow_parking();
    cx.foreground_executor().block_test(task).unwrap()
}

#[gpui::test]
fn mrs_load_within_the_wait_or_are_null_with_their_error_never_empty(cx: &mut TestAppContext) {
    let (_dir, _forge, _, _) = boot(cx);
    let out = ask(
        cx,
        &Identity::owner(),
        "{ workspace { repos { name mrs { id title state } } } }",
    );
    let repos = &out["data"]["workspace"]["repos"];
    assert_eq!(repos[0]["name"], "a");
    assert_eq!(
        repos[0]["mrs"],
        json!([{"id": "2", "title": "Two", "state": "OPENED"},
            {"id": "1", "title": "One", "state": "OPENED"}])
    );
    assert_eq!(repos[1]["mrs"], Value::Null);
    let errors = out["errors"].as_array().unwrap();
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert_eq!(errors[0]["path"], json!(["workspace", "repos", 1, "mrs"]));
    let said = errors[0]["message"].as_str().unwrap();
    assert!(
        said.starts_with("b's merge requests: Choose the remote"),
        "{said}"
    );
}

#[gpui::test]
fn a_subject_reads_its_repo_alone_and_its_mr_alone(cx: &mut TestAppContext) {
    let (_dir, _forge, a, b) = boot(cx);
    let q = format!(
        r#"{{ workspace {{ repos {{ name mrs {{ id }} }} current {{ name }} }}
            other: repo(path: "{b}") {{ name }} mine: repo(path: "{a}") {{ name }} }}"#
    );
    let out = ask(cx, &Identity::subject(&a, Some(2)), &q);
    assert_eq!(
        out["data"],
        json!({"workspace": {"repos": [{"name": "a", "mrs": [{"id": "2"}]}],
            "current": {"name": "a"}}, "other": null, "mine": {"name": "a"}})
    );
    let out = ask(cx, &Identity::subject(&b, None), &q);
    assert_eq!(
        out["data"],
        json!({"workspace": {"repos": [{"name": "b", "mrs": null}], "current": null},
            "other": {"name": "b"}, "mine": null})
    );
    let out = ask(
        cx,
        &Identity::subject(&a, None),
        &format!(r#"{{ repo(path: "{a}") {{ mrs {{ id }} }} }}"#),
    );
    assert_eq!(out["data"]["repo"]["mrs"], json!([]));
    let out = ask(cx, &Identity::owner(), &q);
    assert_eq!(
        out["data"]["workspace"]["repos"].as_array().unwrap().len(),
        2
    );
}

#[gpui::test]
fn a_repo_s_skills_load_within_the_wait_for_the_owner_and_its_subject(cx: &mut TestAppContext) {
    let (dir, _forge, a, _) = boot(cx);
    let lint = dir.path().join("home/.agents/skills/lint");
    std::fs::create_dir_all(&lint).unwrap();
    std::fs::write(lint.join("SKILL.md"), "---\ndescription: Lints\n---\nLint.").unwrap();
    let q = format!(r#"{{ repo(path: "{a}") {{ skills {{ name source description path }} }} }}"#);
    for who in [Identity::owner(), Identity::subject(&a, None)] {
        let out = ask(cx, &who, &q);
        assert_eq!(
            out["data"]["repo"]["skills"],
            json!([{"name": "lint", "source": "AGENTS", "description": "Lints", "path": lint}])
        );
    }
}
