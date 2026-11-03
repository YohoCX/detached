//! Serves the app's commands over HTTP for the Playwright suite in `e2e/`.
//! `POST /reset {"root"}` starts a fresh state on a fixture dir; `POST /invoke {"cmd","args"}` answers
//! with NDJSON: `{"event"}` per channel message as it is sent, then `{"ok"}` or `{"err"}`.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::{Arc, Mutex, RwLock};

use detached_lib::accounts::{self, store::MemoryStore};
use detached_lib::env::{Env, which};
use detached_lib::forge;
use detached_lib::git::{branches, checkout, diff, files, log, pull, remotes};
use detached_lib::settings::Settings;
use detached_lib::state::AppState;
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

/// Git runs through a wrapper with HOME and global config in the fixture, like `git::testutil`.
fn reset(root: &Path) -> Result<Value, String> {
    let path = std::env::var("PATH").unwrap_or_default();
    let real = which("git", &path).ok_or("git not found")?;
    let git = root.join("git");
    let h = root.join("home");
    let h = h.display();
    std::fs::write(
        &git,
        format!(
            "#!/bin/sh\nexport HOME='{h}' GIT_CONFIG_GLOBAL='{h}/.gitconfig' GIT_CONFIG_NOSYSTEM=1\n\
             unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE\nexec '{}' \"$@\"\n",
            real.display()
        ),
    )
    .and_then(|_| std::fs::set_permissions(&git, std::fs::Permissions::from_mode(0o755)))
    .map_err(|e| e.to_string())?;
    let config_dir = root.join("config");
    let settings = Settings::load(&config_dir);
    let tokens = Arc::new(MemoryStore::default());
    let state = AppState::new(config_dir, settings, Env { path, git }, tokens);
    *STATE.write().unwrap() = Some(Arc::new(state));
    Ok(Value::Null)
}

fn arg<T: DeserializeOwned>(args: &Value, k: &str) -> Result<T, String> {
    serde_json::from_value(args.get(k).cloned().unwrap_or_default())
        .map_err(|e| format!("{k}: {e}"))
}

fn ok<T: Serialize>(r: detached_lib::error::Result<T>) -> Result<Value, String> {
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
    let takes = |names: &[&str]| names.contains(&cmd);
    let no_repo = [
        "get_settings",
        "list_repos",
        "add_folder",
        "remove_folder",
        "exclude_repo",
        "include_repo",
        "set_workspace_active",
        "save_workspace",
        "remove_workspace",
        "env_info",
        "keymap_overrides",
        "cancel_op",
        "list_accounts",
        "save_account",
        "remove_account",
        "verify_account",
        "quick_open",
        "find_in_files",
        "cancel_find",
        "cancel_log",
    ];
    let repo = if takes(&no_repo) {
        String::new()
    } else {
        st("repo")?
    };
    let rp = Path::new(&repo);
    let op_id = if takes(&[
        "delete_remote_branch",
        "prune",
        "fetch",
        "pull",
        "cancel_op",
        "fetch_mr",
    ]) {
        arg(a, "opId")?
    } else {
        0
    };
    let progress = |l: &str| emit(json!(l));
    match cmd {
        "get_settings" => Ok(json!(s.get_settings())),
        "list_repos" => Ok(json!(s.list_repos())),
        "add_folder" => ok(s.add_folder(st("workspace")?, st("path")?)),
        "remove_folder" => ok(s.remove_folder(st("workspace")?, st("path")?)),
        "exclude_repo" => ok(s.exclude_repo(st("workspace")?, st("path")?)),
        "include_repo" => ok(s.include_repo(st("workspace")?, st("path")?)),
        "set_workspace_active" => {
            ok(s.set_workspace_active(st("name")?, arg::<bool>(a, "active")?))
        }
        "save_workspace" => ok(s.save_workspace(st("name")?, opt("from")?)),
        "remove_workspace" => ok(s.remove_workspace(st("name")?)),
        "set_repo_account" => ok(s.set_repo_account(repo, opt("accountId")?)),
        "set_forge_account" => ok(s.set_forge_account(repo, st("host")?, opt("accountId")?)),
        "set_mr_remote" => ok(s.set_mr_remote(repo, opt("remote")?)),
        "env_info" => Ok(json!(s.env().info())),
        "keymap_overrides" => Ok(json!(s.keymap_overrides())),
        "repo_status" => ok(s.repo_status(&repo)),
        "list_branches" => ok(branches::list(s, rp)),
        "checkout" => ok(checkout::checkout(s, rp, &arg(a, "target")?)),
        "create_branch" => ok(branches::create(
            s,
            rp,
            &st("name")?,
            &st("start")?,
            arg(a, "checkout")?,
        )),
        "set_upstream" => ok(branches::set_upstream(
            s,
            rp,
            &st("branch")?,
            opt("upstream")?.as_deref(),
        )),
        "delete_branch" => ok(branches::delete(s, rp, &st("name")?, arg(a, "force")?)),
        "delete_remote_branch" => {
            let (remote, name) = (st("remote")?, st("name")?);
            ok(s.op(op_id, |c| {
                branches::delete_remote(s, rp, &remote, &name, c, &progress)
            }))
        }
        "prune" => ok(s.op(op_id, |c| branches::prune(s, rp, c, &progress))),
        "fetch" => ok(s.op(op_id, |c| pull::fetch(s, rp, c, &progress))),
        "pull" => ok(s.op(op_id, |c| pull::pull(s, rp, c, &progress))),
        "cancel_op" => Ok(json!(s.ops.cancel(op_id))),
        "list_changes" => ok(diff::list_changes(
            &s.env(),
            rp,
            &arg(a, "from")?,
            &arg(a, "to")?,
        )),
        "compare_base" => ok(diff::compare_base(&s.env(), rp, opt("base")?.as_deref())),
        "file_diff" => {
            let (left, right) = (arg(a, "left")?, arg(a, "right")?);
            ok(diff::file_diff(
                s,
                rp,
                &st("path")?,
                opt("origPath")?.as_deref(),
                &left,
                &right,
            ))
        }
        "list_files" => ok(files::list_files(&s.env(), rp, opt("rev")?.as_deref())),
        "read_file" => ok(s.read_file(&repo, opt("rev")?, &st("path")?)),
        "read_image" => ok(diff::image(s, rp, opt("rev")?.as_deref(), &st("path")?)),
        "log" => {
            let (query, id) = (arg(a, "query")?, arg(a, "queryId")?);
            s.op(id, |c| {
                log::log(&s.env(), rp, &query, id, c, &|e| emit(json!(e)))
            });
            Ok(Value::Null)
        }
        "cancel_log" => Ok(json!(s.ops.cancel(arg(a, "queryId")?))),
        "commit_details" => ok(log::commit_details(&s.env(), rp, &st("sha")?)),
        "branches_containing" => ok(log::branches_containing(&s.env(), rp, &st("sha")?)),
        "merge_base" => ok(log::merge_base(&s.env(), rp, &st("a")?, &st("b")?)),
        "range_log" => ok(log::range_log(&s.env(), rp, &st("base")?, &st("head")?)),
        "hunk_ranges" => ok(log::hunk_ranges(
            &s.env(),
            rp,
            &st("base")?,
            &st("head")?,
            &st("path")?,
            opt("origPath")?.as_deref(),
        )),
        "fetch_mr" => {
            let (remote, head, target) = (st("remote")?, st("headRef")?, st("targetBranch")?);
            ok(s.op(op_id, |c| {
                remotes::fetch_mr(s, rp, &remote, &head, &target, c, &progress)
            }))
        }
        "list_accounts" => ok(accounts::load(&s.config_dir)),
        "save_account" => ok(accounts::save(s, arg(a, "input")?)),
        "remove_account" => ok(accounts::remove(s, &st("id")?)),
        "verify_account" => ok(accounts::verify(s, &st("id")?)),
        "repo_accounts" => ok(accounts::repo_accounts(s, rp)),
        "quick_open" => {
            let repos: Vec<String> = arg(a, "repos")?;
            ok(s.quick_open(
                &repos,
                opt("rev")?.as_deref(),
                &st("query")?,
                arg(a, "limit")?,
            ))
        }
        "find_in_files" => {
            let repos: Vec<String> = arg(a, "repos")?;
            let (query, id) = (arg(a, "query")?, arg(a, "queryId")?);
            s.find_in_files(&repos, opt("rev")?.as_deref(), &query, id, &|e| {
                emit(json!(e))
            });
            Ok(Value::Null)
        }
        "cancel_find" => Ok(json!(s.ops.cancel(arg(a, "queryId")?))),
        "forge_hosts" => ok(forge::forge_hosts(s, rp)),
        "commit_url" => ok(forge::commit_url(s, rp, &st("sha")?)),
        "list_mrs" => ok(forge::list_mrs(s, rp, arg(a, "filter")?)),
        "current_mr" => ok(forge::current_mr(s, rp)),
        "get_mr" => ok(forge::get_mr(s, rp, arg(a, "number")?)),
        "mr_threads" => ok(forge::mr_threads(s, rp, arg(a, "number")?)),
        "mr_action" => ok(forge::mr_action(
            s,
            rp,
            arg(a, "number")?,
            &arg(a, "action")?,
        )),
        "new_mr" => ok(forge::new_mr(s, rp)),
        "create_mr" => ok(forge::create_mr(s, rp, &arg(a, "input")?)),
        "list_pipelines" => ok(forge::list_pipelines(s, rp, arg(a, "all")?)),
        "mr_pipelines" => ok(forge::mr_pipelines(s, rp, arg(a, "number")?)),
        "pipeline_jobs" => ok(forge::pipeline_jobs(
            s,
            rp,
            &arg(a, "project")?,
            arg(a, "id")?,
        )),
        "pipeline_action" => ok(forge::pipeline_action(
            s,
            rp,
            &arg(a, "project")?,
            arg(a, "id")?,
            arg(a, "action")?,
        )),
        "job_action" => ok(forge::job_action(
            s,
            rp,
            &arg(a, "project")?,
            arg(a, "id")?,
            arg(a, "action")?,
        )),
        "job_log" => ok(forge::job_log(
            s,
            rp,
            &arg(a, "project")?,
            arg(a, "id")?,
            arg(a, "offset")?,
        )),
        "workflows" => ok(forge::workflows(s, rp, &arg(a, "project")?)),
        "run_pipeline" => ok(forge::run_pipeline(s, rp, &arg(a, "input")?)),
        _ => Err(format!("{cmd} is not served by the e2e bridge")),
    }
}
