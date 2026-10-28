use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex, RwLock};

use tempfile::TempDir;

use super::*;
use crate::accounts::store::MemoryStore;
use crate::env::{Env, which};
use crate::git::runner::{self, Kind, Opts};
use crate::settings::Settings;
use crate::types::Provider;

fn acc(id: &str, base_url: &str) -> Account {
    Account {
        id: id.into(),
        provider: Provider::GitlabSelfHosted,
        base_url: base_url.into(),
        username: format!("user-{id}"),
        label: id.into(),
    }
}

fn state(accounts: &[Account]) -> (TempDir, AppState) {
    let dir = tempfile::tempdir().unwrap();
    store(dir.path(), accounts).unwrap();
    let path = std::env::var("PATH").unwrap();
    let state = AppState {
        config_dir: dir.path().into(),
        settings: Mutex::new(Settings::default()),
        env: RwLock::new(Env {
            git: which("git", &path).unwrap(),
            path,
        }),
        ops: Default::default(),
        locks: Default::default(),
        tokens: Arc::new(MemoryStore::default()),
    };
    (dir, state)
}

fn set_override(state: &AppState, repo: &str, id: &str) {
    let mut s = state.settings.lock().unwrap();
    s.repo_accounts.insert(repo.into(), id.into());
}

fn matched(
    accounts: &[Account],
    over: Option<&str>,
    url: &str,
) -> Option<(String, Option<String>, bool)> {
    resolve(accounts, over, url).map(|(h, a, amb)| (h, a.map(|a| a.id.clone()), amb))
}

#[test]
fn parses_https_urls_only() {
    let p = https;
    assert_eq!(
        p("https://user@Git.Example.com:8443/a/b.git"),
        Some(("git.example.com:8443".into(), "/a/b.git".into()))
    );
    assert_eq!(
        p("https://github.com/"),
        Some(("github.com".into(), "".into()))
    );
    assert_eq!(p("git@github.com:a/b.git"), None);
    assert_eq!(p("ssh://git@github.com/a/b.git"), None);
    assert_eq!(p("http://github.com/a/b.git"), None);
}

#[test]
fn matches_single_ambiguous_override_port_and_sub_path() {
    let one = [acc("a", "https://github.com")];
    let some = |id: &str| Some(id.to_string());
    assert_eq!(
        matched(&one, None, "https://github.com/x/y.git"),
        Some(("github.com".into(), some("a"), false))
    );
    assert_eq!(matched(&one, None, "git@github.com:x/y.git"), None);
    assert_eq!(
        matched(&one, None, "https://gitlab.com/x/y.git"),
        Some(("gitlab.com".into(), None, false))
    );

    let two = [
        acc("a", "https://github.com"),
        acc("b", "https://github.com"),
    ];
    assert_eq!(
        matched(&two, None, "https://github.com/x"),
        Some(("github.com".into(), None, true))
    );
    assert_eq!(
        matched(&two, Some("b"), "https://github.com/x"),
        Some(("github.com".into(), some("b"), false))
    );
    // An override for another host does not apply here.
    let mixed = [
        acc("a", "https://github.com"),
        acc("c", "https://gitlab.com"),
    ];
    assert_eq!(
        matched(&mixed, Some("c"), "https://github.com/x"),
        Some(("github.com".into(), some("a"), false))
    );

    let ports = [
        acc("a", "https://git.example.com"),
        acc("b", "https://git.example.com:8443"),
    ];
    assert_eq!(
        matched(&ports, None, "https://git.example.com:8443/x")
            .unwrap()
            .1,
        some("b")
    );
    assert_eq!(
        matched(&ports, None, "https://git.example.com/x")
            .unwrap()
            .1,
        some("a")
    );

    let sub = [acc("a", "https://git.example.com/gitlab")];
    assert_eq!(
        matched(&sub, None, "https://git.example.com/gitlab/g/r.git")
            .unwrap()
            .1,
        some("a")
    );
    assert_eq!(
        matched(&sub, None, "https://git.example.com/gitlabx/r.git")
            .unwrap()
            .1,
        None
    );
    assert_eq!(
        matched(&sub, None, "https://git.example.com/other/r.git")
            .unwrap()
            .1,
        None
    );
}

#[test]
fn credentials_per_host_keep_secrets_out_of_args() {
    let (_d, state) = state(&[
        acc("a", "https://github.com"),
        acc("b", "https://git.example.com"),
        acc("c", "https://nokey.example.com"),
    ]);
    state.tokens.set("a", "tok-a").unwrap();
    state.tokens.set("b", "tok-b").unwrap();
    let urls = [
        "https://github.com/x/y.git",
        "https://github.com/x/z.git",
        "git@git.example.com:x/y.git",
        "https://nokey.example.com/x",
        "https://git.example.com/x",
    ]
    .map(String::from);
    let cred = credentials(&state, Path::new("/r"), &urls)
        .unwrap()
        .unwrap();
    let helper = |host: &str, n: u8| {
        format!(
            r#"credential.https://{host}.helper=!f() {{ test "$1" = get && printf "username=%s\npassword=%s\n" "$WB_USER_{n}" "$WB_TOKEN_{n}"; }}; f"#
        )
    };
    assert_eq!(
        cred.args,
        [
            "-c".into(),
            "credential.https://github.com.helper=".into(),
            "-c".into(),
            helper("github.com", 0),
            "-c".into(),
            "credential.https://git.example.com.helper=".into(),
            "-c".into(),
            helper("git.example.com", 1),
        ]
    );
    let env = |k: &str, v: &str| (k.to_string(), v.to_string());
    assert_eq!(
        cred.env,
        [
            env("WB_USER_0", "user-a"),
            env("WB_TOKEN_0", "tok-a"),
            env("WB_USER_1", "user-b"),
            env("WB_TOKEN_1", "tok-b")
        ]
    );

    let ssh = ["git@github.com:x/y.git".to_string()];
    assert_eq!(credentials(&state, Path::new("/r"), &ssh).unwrap(), None);
}

#[test]
fn ambiguous_host_gets_no_credentials_until_overridden() {
    let (_d, state) = state(&[
        acc("a", "https://github.com"),
        acc("b", "https://github.com"),
    ]);
    state.tokens.set("a", "tok-a").unwrap();
    state.tokens.set("b", "tok-b").unwrap();
    let urls = ["https://github.com/x".to_string()];
    assert_eq!(credentials(&state, Path::new("/r"), &urls).unwrap(), None);
    set_override(&state, "/r", "b");
    let cred = credentials(&state, Path::new("/r"), &urls)
        .unwrap()
        .unwrap();
    assert_eq!(cred.env[1], ("WB_TOKEN_0".into(), "tok-b".into()));
    assert_eq!(
        credentials(&state, Path::new("/other"), &urls).unwrap(),
        None
    );
}

#[test]
fn git_uses_app_token_only_for_its_host() {
    let host = "git.example.com:8443";
    let (_d, state) = state(&[acc("a", &format!("https://{host}"))]);
    state.tokens.set("a", "tok-secret").unwrap();
    let urls = [format!("https://{host}/g/r.git")];
    let mut cred = credentials(&state, Path::new("/r"), &urls)
        .unwrap()
        .unwrap();

    let home = tempfile::tempdir().unwrap();
    let log = home.path().join("helper.log");
    let rec = home.path().join("record.sh");
    let script = format!(
        "#!/bin/sh\necho \"$1\" >> '{}'\ncat >/dev/null\ntest \"$1\" = get && printf 'username=theirs\\npassword=user-helper\\n'\nexit 0\n",
        log.display()
    );
    std::fs::write(&rec, script).unwrap();
    std::fs::set_permissions(&rec, std::fs::Permissions::from_mode(0o755)).unwrap();
    let global = home.path().join("gitconfig");
    std::fs::write(
        &global,
        format!("[credential]\n\thelper = {}\n", rec.display()),
    )
    .unwrap();
    for (k, v) in [
        ("HOME", home.path()),
        ("GIT_CONFIG_GLOBAL", &global),
        ("GIT_CONFIG_NOSYSTEM", Path::new("1")),
    ] {
        cred.env.push((k.into(), v.to_str().unwrap().into()));
    }

    // The runner gives git no stdin, so a shell alias feeds `git credential`.
    let credential = |op: &str, input: &str| {
        let alias = format!("alias.t=!printf '{input}' | git credential {op}");
        let opts = Opts {
            cred: Some(&cred),
            ..Default::default()
        };
        let out = runner::run(
            &state.env(),
            home.path(),
            &["-c", &alias, "t"],
            Kind::Read,
            opts,
        )
        .unwrap();
        assert_eq!(out.code, 0, "{}", out.stderr);
        out.text()
    };
    let calls = || std::fs::read_to_string(&log).unwrap_or_default();

    let out = credential("fill", &format!("url=https://{host}/g/r.git\\n\\n"));
    assert!(
        out.contains("username=user-a\npassword=tok-secret\n"),
        "{out}"
    );
    credential(
        "approve",
        &format!("url=https://{host}\\nusername=user-a\\npassword=tok-secret\\n\\n"),
    );
    assert_eq!(calls(), "");

    let out = credential("fill", "url=https://other.example.com/r.git\\n\\n");
    assert!(
        out.contains("password=user-helper") && !out.contains("tok-secret"),
        "{out}"
    );
    assert_eq!(calls(), "get\n");
}

#[test]
fn save_update_and_remove() {
    let (_d, state) = state(&[]);
    let input = |id: Option<String>, token: Option<&str>| AccountInput {
        id,
        provider: Provider::GitlabSelfHosted,
        base_url: " https://git.example.com/gitlab/ ".into(),
        username: "oauth2".into(),
        label: "Work".into(),
        token: token.map(String::from),
    };
    let a = save(&state, input(None, Some(" tok "))).unwrap();
    assert_eq!(a.base_url, "https://git.example.com/gitlab");
    assert_eq!(state.tokens.get(&a.id).unwrap().as_deref(), Some("tok"));
    let b = save(&state, input(None, None)).unwrap();
    assert_ne!(a.id, b.id);

    let mut edit = input(Some(a.id.clone()), None);
    edit.label = "Renamed".into();
    save(&state, edit).unwrap();
    assert_eq!(state.tokens.get(&a.id).unwrap().as_deref(), Some("tok"));
    let all = load(&state.config_dir).unwrap();
    assert_eq!((all.len(), all[0].label.as_str()), (2, "Renamed"));

    assert!(save(&state, input(Some("missing".into()), None)).is_err());
    let mut http = input(None, None);
    http.base_url = "http://git.example.com".into();
    assert!(save(&state, http).is_err());

    set_override(&state, "/r1", &a.id);
    set_override(&state, "/r2", &b.id);
    remove(&state, &a.id).unwrap();
    assert_eq!(load(&state.config_dir).unwrap(), vec![b.clone()]);
    assert_eq!(state.tokens.get(&a.id).unwrap(), None);
    let saved = Settings::load(&state.config_dir);
    assert_eq!(
        saved.repo_accounts.into_iter().collect::<Vec<_>>(),
        [("/r2".to_string(), b.id.clone())]
    );
    remove(&state, "never-existed").unwrap();
}

#[test]
fn verify_without_token_fails_without_network() {
    let (_d, state) = state(&[acc("a", "https://git.example.com")]);
    assert!(matches!(
        verify(&state, "a").unwrap(),
        VerifyResult::Failed { status: None, .. }
    ));
    assert!(verify(&state, "missing").is_err());
}

#[test]
fn repo_accounts_groups_remotes_by_host() {
    let (_d, state) = state(&[
        acc("a", "https://github.com"),
        acc("b", "https://gitlab.com"),
        acc("c", "https://gitlab.com"),
    ]);
    let repo = tempfile::tempdir().unwrap();
    let git = |args: &[&str]| {
        let ok = Command::new("git")
            .arg("-C")
            .arg(repo.path())
            .args(args)
            .env("HOME", repo.path())
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .status()
            .unwrap()
            .success();
        assert!(ok);
    };
    git(&["init", "-q"]);
    assert_eq!(repo_accounts(&state, repo.path()).unwrap(), vec![]);
    for (name, url) in [
        ("origin", "https://github.com/x/y.git"),
        ("ssh", "git@github.com:x/y.git"),
        ("fork", "https://me@github.com/me/y.git"),
        ("lab", "https://gitlab.com/x/y.git"),
        ("short", "gh:x/z.git"),
    ] {
        git(&["remote", "add", name, url]);
    }
    git(&["config", "url.https://github.com/.insteadOf", "gh:"]);
    let host = |host: &str, remotes: &[&str], account_id: Option<&str>, ambiguous| HostAccount {
        host: host.into(),
        remotes: remotes.iter().map(|r| r.to_string()).collect(),
        account_id: account_id.map(String::from),
        ambiguous,
    };
    assert_eq!(
        repo_accounts(&state, repo.path()).unwrap(),
        [
            host("github.com", &["fork", "origin", "short"], Some("a"), false),
            host("gitlab.com", &["lab"], None, true)
        ]
    );
    set_override(&state, repo.path().to_str().unwrap(), "c");
    assert_eq!(
        repo_accounts(&state, repo.path()).unwrap()[1],
        host("gitlab.com", &["lab"], Some("c"), false)
    );
}
