//! Serves the app's commands over HTTP for the Playwright suite in `e2e/`.
//! `POST /reset {"root"}` starts a fresh state on a fixture dir; `POST /invoke {"cmd","args"}` answers
//! with NDJSON: `{"event"}` per channel message as it is sent, then `{"ok"}` or `{"err"}`.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::sync::{Arc, Mutex, RwLock};

use detached_core::accounts::store::MemoryStore;
use detached_core::api;
use detached_core::env::{Env, find_git};
use detached_core::settings::Settings;
use detached_core::state::AppState;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

static STATE: RwLock<Option<Arc<AppState>>> = RwLock::new(None);

fn main() {
    let port = std::env::args().nth(1).unwrap_or("1521".into());
    let listener = TcpListener::bind(format!("127.0.0.1:{port}")).unwrap();
    for stream in listener.incoming().flatten() {
        std::thread::spawn(|| serve(stream));
    }
}

fn serve(stream: TcpStream) {
    let mut reader = BufReader::new(&stream);
    let mut line = String::new();
    let _ = reader.read_line(&mut line);
    let mut len = 0;
    loop {
        let mut h = String::new();
        if reader.read_line(&mut h).unwrap_or(0) == 0 || h == "\r\n" {
            break;
        }
        if let Some((k, v)) = h.split_once(':')
            && k.eq_ignore_ascii_case("content-length")
        {
            len = v.trim().parse().unwrap_or(0);
        }
    }
    let mut body = vec![0; len];
    let _ = reader.read_exact(&mut body);
    let out = Mutex::new(&stream);
    let _ = out.lock().unwrap().write_all(
        b"HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\n\
          Access-Control-Allow-Origin: *\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n",
    );
    let send = |v: Value| {
        let line = format!("{v}\n");
        let mut s = out.lock().unwrap();
        let _ = write!(s, "{:x}\r\n{line}\r\n", line.len()).and_then(|_| s.flush());
    };
    let req: Value = serde_json::from_slice(&body).unwrap_or_default();
    let r = match line.split(' ').nth(1).unwrap_or_default() {
        "/reset" => reset(Path::new(req["root"].as_str().unwrap_or_default())),
        "/invoke" => match STATE.read().unwrap().clone() {
            Some(s) => invoke(
                &s,
                req["cmd"].as_str().unwrap_or_default(),
                &req["args"],
                &|e| send(json!({ "event": e })),
            ),
            None => Err("no fixture: POST /reset first".into()),
        },
        _ => Ok(Value::Null),
    };
    send(match r {
        Ok(v) => json!({ "ok": v }),
        Err(e) => json!({ "err": e }),
    });
    let _ = out.lock().unwrap().write_all(b"0\r\n\r\n");
}

/// Git runs with HOME and global config in the fixture, like `git::testutil`.
fn reset(root: &Path) -> Result<Value, String> {
    let path = std::env::var("PATH").unwrap_or_default();
    let git = find_git(&path).ok_or("git not found")?;
    let home = root.join("home").display().to_string();
    let vars = vec![
        ("HOME".into(), home.clone()),
        ("GIT_CONFIG_GLOBAL".into(), format!("{home}/.gitconfig")),
        ("GIT_CONFIG_NOSYSTEM".into(), "1".into()),
    ];
    let config_dir = root.join("config");
    let settings = Settings::load(&config_dir);
    let tokens = Arc::new(MemoryStore::default());
    let state = AppState::new(config_dir, settings, Env { path, git, vars }, tokens);
    *STATE.write().unwrap() = Some(Arc::new(state));
    Ok(Value::Null)
}

fn arg<T: DeserializeOwned>(args: &Value, k: &str) -> Result<T, String> {
    serde_json::from_value(args.get(k).cloned().unwrap_or_default())
        .map_err(|e| format!("{k}: {e}"))
}

fn ok<T: Serialize>(r: detached_core::error::Result<T>) -> Result<Value, String> {
    r.map(|v| json!(v)).map_err(|e| e.to_string())
}

/// The same calls the Tauri commands make. Open in Finder/Zed and the git path setting are left
/// out: they start other apps or a login shell.
fn invoke(
    s: &AppState,
    cmd: &str,
    a: &Value,
    emit: &(dyn Fn(Value) + Sync),
) -> Result<Value, String> {
    let st = |k| arg::<String>(a, k);
    let opt = |k| arg::<Option<String>>(a, k);
    // Required as Tauri requires them; an empty repo would run git in the bridge's cwd, the checkout.
    let repo = || st("repo");
    let progress = |l: &str| emit(json!(l));
    match cmd {
        "get_settings" => Ok(json!(api::get_settings(s))),
        "list_repos" => Ok(json!(api::list_repos(s))),
        "add_folder" => ok(api::add_folder(s, st("workspace")?, st("path")?)),
        "remove_folder" => ok(api::remove_folder(s, st("workspace")?, st("path")?)),
        "exclude_repo" => ok(api::exclude_repo(s, st("workspace")?, st("path")?)),
        "include_repo" => ok(api::include_repo(s, st("workspace")?, st("path")?)),
        "set_workspace_active" => ok(api::set_workspace_active(s, st("name")?, arg(a, "active")?)),
        "save_workspace" => ok(api::save_workspace(s, st("name")?, opt("from")?)),
        "remove_workspace" => ok(api::remove_workspace(s, st("name")?)),
        "set_repo_account" => ok(api::set_repo_account(s, repo()?, opt("accountId")?)),
        "set_forge_account" => ok(api::set_forge_account(
            s,
            repo()?,
            st("host")?,
            opt("accountId")?,
        )),
        "set_mr_remote" => ok(api::set_mr_remote(s, repo()?, opt("remote")?)),
        "env_info" => Ok(json!(api::env_info(s))),
        "keymap_overrides" => Ok(json!(api::keymap_overrides(s))),
        "repo_status" => ok(api::repo_status(s, &repo()?)),
        "list_branches" => ok(api::list_branches(s, &repo()?)),
        "checkout" => ok(api::checkout(s, &repo()?, &arg(a, "target")?)),
        "create_branch" => ok(api::create_branch(
            s,
            &repo()?,
            &st("name")?,
            &st("start")?,
            arg(a, "checkout")?,
        )),
        "set_upstream" => ok(api::set_upstream(
            s,
            &repo()?,
            &st("branch")?,
            opt("upstream")?.as_deref(),
        )),
        "delete_branch" => ok(api::delete_branch(
            s,
            &repo()?,
            &st("name")?,
            arg(a, "force")?,
        )),
        "delete_remote_branch" => ok(api::delete_remote_branch(
            s,
            &repo()?,
            &st("remote")?,
            &st("name")?,
            arg(a, "opId")?,
            &progress,
        )),
        "prune" => ok(api::prune(s, &repo()?, arg(a, "opId")?, &progress)),
        "fetch" => ok(api::fetch(s, &repo()?, arg(a, "opId")?, &progress)),
        "pull" => ok(api::pull(s, &repo()?, arg(a, "opId")?, &progress)),
        "cancel_op" => Ok(json!(api::cancel(s, arg(a, "opId")?))),
        "list_changes" => ok(api::list_changes(
            s,
            &repo()?,
            &arg(a, "from")?,
            &arg(a, "to")?,
        )),
        "compare_base" => ok(api::compare_base(s, &repo()?, opt("base")?.as_deref())),
        "file_diff" => ok(api::file_diff(
            s,
            &repo()?,
            &st("path")?,
            opt("origPath")?.as_deref(),
            &arg(a, "left")?,
            &arg(a, "right")?,
        )),
        "list_files" => ok(api::list_files(s, &repo()?, opt("rev")?.as_deref())),
        "read_file" => ok(api::read_file(
            s,
            &repo()?,
            opt("rev")?.as_deref(),
            &st("path")?,
        )),
        "read_image" => ok(api::read_image(
            s,
            &repo()?,
            opt("rev")?.as_deref(),
            &st("path")?,
        )),
        "log" => {
            let (query, id) = (arg(a, "query")?, arg(a, "queryId")?);
            api::log(s, &repo()?, &query, id, &|e| emit(json!(e)));
            Ok(Value::Null)
        }
        "cancel_log" => Ok(json!(api::cancel(s, arg(a, "queryId")?))),
        "commit_details" => ok(api::commit_details(s, &repo()?, &st("sha")?)),
        "branches_containing" => ok(api::branches_containing(s, &repo()?, &st("sha")?)),
        "merge_base" => ok(api::merge_base(s, &repo()?, &st("a")?, &st("b")?)),
        "range_log" => ok(api::range_log(s, &repo()?, &st("base")?, &st("head")?)),
        "hunk_ranges" => ok(api::hunk_ranges(
            s,
            &repo()?,
            &st("base")?,
            &st("head")?,
            &st("path")?,
            opt("origPath")?.as_deref(),
        )),
        "fetch_mr" => ok(api::fetch_mr(
            s,
            &repo()?,
            &st("remote")?,
            &st("headRef")?,
            &st("targetBranch")?,
            arg(a, "opId")?,
            &progress,
        )),
        "list_accounts" => ok(api::list_accounts(s)),
        "save_account" => ok(api::save_account(s, arg(a, "input")?)),
        "remove_account" => ok(api::remove_account(s, &st("id")?)),
        "verify_account" => ok(api::verify_account(s, &st("id")?)),
        "repo_accounts" => ok(api::repo_accounts(s, &repo()?)),
        "quick_open" => ok(api::quick_open(
            s,
            &arg::<Vec<String>>(a, "repos")?,
            opt("rev")?.as_deref(),
            &st("query")?,
            arg(a, "limit")?,
        )),
        "find_in_files" => {
            let (repos, query, id) = (
                arg::<Vec<String>>(a, "repos")?,
                arg(a, "query")?,
                arg(a, "queryId")?,
            );
            let rev = opt("rev")?;
            api::find_in_files(s, &repos, rev.as_deref(), &query, id, &|e| emit(json!(e)));
            Ok(Value::Null)
        }
        "cancel_find" => Ok(json!(api::cancel(s, arg(a, "queryId")?))),
        "forge_hosts" => ok(api::forge_hosts(s, &repo()?)),
        "commit_url" => ok(api::commit_url(s, &repo()?, &st("sha")?)),
        "list_mrs" => ok(api::list_mrs(s, &repo()?, arg(a, "filter")?)),
        "current_mr" => ok(api::current_mr(s, &repo()?)),
        "get_mr" => ok(api::get_mr(s, &repo()?, arg(a, "number")?)),
        "mr_threads" => ok(api::mr_threads(s, &repo()?, arg(a, "number")?)),
        "mr_action" => ok(api::mr_action(
            s,
            &repo()?,
            arg(a, "number")?,
            &arg(a, "action")?,
        )),
        "new_mr" => ok(api::new_mr(s, &repo()?)),
        "create_mr" => ok(api::create_mr(s, &repo()?, &arg(a, "input")?)),
        "list_pipelines" => ok(api::list_pipelines(s, &repo()?, arg(a, "all")?)),
        "mr_pipelines" => ok(api::mr_pipelines(s, &repo()?, arg(a, "number")?)),
        "pipeline_jobs" => ok(api::pipeline_jobs(
            s,
            &repo()?,
            &arg(a, "project")?,
            arg(a, "id")?,
        )),
        "pipeline_action" => ok(api::pipeline_action(
            s,
            &repo()?,
            &arg(a, "project")?,
            arg(a, "id")?,
            arg(a, "action")?,
        )),
        "job_action" => ok(api::job_action(
            s,
            &repo()?,
            &arg(a, "project")?,
            arg(a, "id")?,
            arg(a, "action")?,
        )),
        "job_log" => ok(api::job_log(
            s,
            &repo()?,
            &arg(a, "project")?,
            arg(a, "id")?,
            arg(a, "offset")?,
        )),
        "workflows" => ok(api::workflows(s, &repo()?, &arg(a, "project")?)),
        "run_pipeline" => ok(api::run_pipeline(s, &repo()?, &arg(a, "input")?)),
        _ => Err(format!("{cmd} is not served by the e2e bridge")),
    }
}
