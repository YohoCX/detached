//! Which repos may load their own agent settings: what those would run, and the user's yes to it.
//!
//! A session shows no trust dialog, so a repo's hooks would run unasked.

use std::collections::{BTreeMap, HashSet};
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, mpsc};

use notify::{RecommendedWatcher, RecursiveMode, Watcher as _};
use serde_json::Value;

use crate::env::Env;
use crate::error::{Error, Result};
use crate::git::runner::{Kind, out};

const FILE: &str = "agent-trust.json";
const SETTINGS: &[&str] = &[".claude/settings.json", ".claude/settings.local.json"];
const MARKDOWN: &[&str] = &[
    ".claude/skills",
    ".claude/agents",
    ".claude/commands",
    ".agents/skills",
];
/// Codex's project layer (config, rules, hooks), which no parser here reads: each file is listed
/// whole when it says anything.
const CODEX: &str = ".codex";
/// Settings that run nothing and grant nothing.
const QUIET: &[&str] = &[
    "$schema",
    "model",
    "outputStyle",
    "includeCoAuthoredBy",
    "attribution",
    "language",
    "cleanupPeriodDays",
];
/// Front matter keys that run or grant something.
const FRONT: &[&str] = &["hooks", "allowed-tools", "mcpServers", "permissionMode"];

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Project {
    /// What the repo's settings would run or allow, one line each.
    pub items: Vec<String>,
    /// The files read, with their git object ids; any change asks again.
    pub fingerprint: String,
    /// Why Codex's files (`.codex/`, `.agents/`) were not scanned: a link or their size. Only a
    /// provider that reads them is refused for it.
    pub refused: Option<String>,
}

impl Project {
    /// Why a provider whose trust globs are `globs` may not load this tree.
    pub fn refuses(&self, globs: &[&str]) -> Option<&str> {
        let reads = globs
            .iter()
            .any(|g| CODEX_ONLY.iter().any(|d| g.starts_with(d)));
        self.refused.as_deref().filter(|_| reads)
    }
}

/// Codex's alone of the trust files: a problem in them refuses Codex, not every provider.
const CODEX_ONLY: &[&str] = &[CODEX, ".agents"];

fn refuse_link(p: impl std::fmt::Display) -> String {
    format!("{p} is a link: the scan refuses it")
}

/// Reads what loading `repo`'s own settings would bring in.
pub fn project(env: &Env, repo: &Path) -> Result<Project> {
    let mut files: Vec<String> = SETTINGS
        .iter()
        .chain(&[".mcp.json"])
        .filter(|p| repo.join(p).is_file())
        .map(|p| p.to_string())
        .collect();
    for dir in MARKDOWN {
        markdown(repo, &repo.join(dir), &mut files);
    }
    let mut codex = vec![];
    let size = |f: &String| fs::metadata(repo.join(f)).map_or(0, |m| m.len());
    let refused = match layer(repo, &repo.join(CODEX), &mut codex) {
        Err(e) => Some(e),
        Ok(()) if codex.iter().map(size).sum::<u64>() > 1 << 20 => Some(over()),
        Ok(()) => {
            files.extend(codex);
            None
        }
    };
    files.sort();
    let mut items = vec![];
    for f in &files {
        let text = fs::read_to_string(repo.join(f)).unwrap_or_default();
        scan(f, &text, &mut items);
    }
    let fingerprint = match files.is_empty() {
        true => String::new(),
        false => {
            let args: Vec<&str> = ["hash-object", "--"]
                .into_iter()
                .chain(files.iter().map(String::as_str))
                .collect();
            let ids = out(env, repo, &args, Kind::Read)?;
            files
                .iter()
                .zip(ids.lines())
                .map(|(f, id)| format!("{id} {f}\n"))
                .collect()
        }
    };
    Ok(Project {
        items,
        fingerprint,
        refused,
    })
}

fn over() -> String {
    format!("{CODEX} is over 1 MiB: the scan refuses it")
}

/// What `repo`'s trust files at `sha` would bring in, read as raw blobs: a link among them or more
/// than 1 MiB of them refuses the scan.
pub fn at(env: &Env, repo: &Path, sha: &str) -> Result<Project> {
    use crate::panel::tree;
    let roots = [".claude", ".mcp.json", ".codex", ".agents"];
    let all = tree::entries(env, repo, sha, &roots)?;
    let wanted = |p: &str| {
        SETTINGS.contains(&p)
            || p == ".mcp.json"
            || p.starts_with(&format!("{CODEX}/"))
            || (MARKDOWN.iter().any(|d| p.starts_with(&format!("{d}/"))) && p.ends_with(".md"))
    };
    let link = |p: &str| {
        roots.contains(&p)
            || MARKDOWN
                .iter()
                .any(|d| d.starts_with(p) || p.starts_with(d))
            || wanted(p)
    };
    let codex_only = |p: &str| {
        CODEX_ONLY
            .iter()
            .any(|d| p == *d || p.starts_with(&format!("{d}/")))
    };
    let links = all.iter().filter(|e| e.mode == "120000" && link(&e.path));
    let (theirs, links): (Vec<_>, Vec<_>) = links.partition(|e| codex_only(&e.path));
    if let Some(e) = links.first() {
        return Err(Error::Other(refuse_link(&e.path)));
    }
    let mut files: Vec<&tree::Entry> = all.iter().filter(|e| wanted(&e.path)).collect();
    let codex = files
        .iter()
        .filter(|e| e.path.starts_with(&format!("{CODEX}/")));
    let refused = match theirs.first() {
        Some(e) => Some(refuse_link(&e.path)),
        None => (codex.map(|e| e.size).sum::<u64>() > 1 << 20).then(over),
    };
    if refused.is_some() {
        files.retain(|e| !codex_only(&e.path));
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    if files.iter().map(|e| e.size).sum::<u64>() > 1 << 20 {
        return Err(Error::Other(
            "The trust files are over 1 MiB: the scan refuses them".into(),
        ));
    }
    let oids: Vec<&str> = files.iter().map(|e| e.oid.as_str()).collect();
    let bodies = tree::blobs(env, repo, &oids)?;
    let mut items = vec![];
    let mut fingerprint = String::new();
    for (e, body) in files.iter().zip(bodies) {
        scan(&e.path, &String::from_utf8_lossy(&body), &mut items);
        fingerprint.push_str(&format!("{} {}\n", e.oid, e.path));
    }
    Ok(Project {
        items,
        fingerprint,
        refused,
    })
}

/// `p` with `repo`'s file `rel` as it is on disk, when it exists, in the order `project` reads.
pub fn with_file(env: &Env, mut p: Project, repo: &Path, rel: &str) -> Result<Project> {
    let tracked = p
        .fingerprint
        .lines()
        .any(|l| l.split_once(' ').is_some_and(|x| x.1 == rel));
    // A tracked one is among the head's files already.
    let text = (!tracked).then(|| fs::read_to_string(repo.join(rel)).ok());
    let Some(text) = text.flatten() else {
        return Ok(p);
    };
    settings(rel, &text, &mut p.items);
    let id = out(env, repo, &["hash-object", "--", rel], Kind::Read)?;
    let mut lines: Vec<String> = p.fingerprint.lines().map(|l| format!("{l}\n")).collect();
    lines.push(format!("{} {rel}\n", id.trim()));
    lines.sort_by(|a, b| {
        a.split_once(' ')
            .map(|x| x.1)
            .cmp(&b.split_once(' ').map(|x| x.1))
    });
    p.fingerprint = lines.concat();
    Ok(p)
}

/// True when there is nothing to trust, or the user trusted exactly this.
pub fn trusted(config_dir: &Path, repo: &str, p: &Project) -> bool {
    p.items.is_empty() || load(config_dir).get(repo) == Some(&p.fingerprint)
}

pub fn trust(config_dir: &Path, repo: &str, p: &Project) -> Result<()> {
    let mut all = load(config_dir);
    all.insert(repo.into(), p.fingerprint.clone());
    fs::create_dir_all(config_dir)?;
    let tmp = config_dir.join(format!("{FILE}.tmp"));
    fs::write(&tmp, serde_json::to_vec_pretty(&all)?)?;
    fs::rename(tmp, config_dir.join(FILE))?;
    Ok(())
}

fn load(config_dir: &Path) -> BTreeMap<String, String> {
    fs::read(config_dir.join(FILE))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

/// Every file under Codex's layer `dir`, read whole: a link in it, or it a link, refuses the scan.
fn layer(repo: &Path, dir: &Path, files: &mut Vec<String>) -> std::result::Result<(), String> {
    let Ok(m) = dir.symlink_metadata() else {
        return Ok(());
    };
    if m.file_type().is_symlink() {
        return Err(refuse_link(dir.display()));
    }
    for e in fs::read_dir(dir).map_err(|e| e.to_string())?.flatten() {
        let (path, kind) = (e.path(), e.file_type().map_err(|e| e.to_string())?);
        if kind.is_symlink() {
            return Err(refuse_link(path.display()));
        } else if kind.is_dir() {
            layer(repo, &path, files)?;
        } else if let Some(rel) = path.strip_prefix(repo).ok().and_then(|p| p.to_str()) {
            files.push(crate::git::files::slashed(rel));
        }
    }
    Ok(())
}

fn markdown(repo: &Path, dir: &Path, files: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let path: PathBuf = e.path();
        if path.is_dir() {
            markdown(repo, &path, files);
        } else if path.extension().is_some_and(|x| x == "md")
            && let Some(rel) = path.strip_prefix(repo).ok().and_then(|p| p.to_str())
        {
            files.push(crate::git::files::slashed(rel));
        }
    }
}

/// What the trust file `file` would bring in.
fn scan(file: &str, text: &str, items: &mut Vec<String>) {
    match file {
        ".mcp.json" => mcp(text, items),
        f if f.starts_with(&format!("{CODEX}/")) => {
            if !text.trim().is_empty() {
                items.push(format!("{f}: unparsed"));
            }
        }
        f if f.ends_with(".json") => settings(f, text, items),
        f => front(f, text, items),
    }
}

fn settings(file: &str, text: &str, items: &mut Vec<String>) {
    let Ok(Value::Object(s)) = serde_json::from_str::<Value>(text) else {
        items.push(format!("{file}: does not parse, so it can't be listed"));
        return;
    };
    for (key, v) in &s {
        match key.as_str() {
            k if QUIET.contains(&k) => {}
            "hooks" => hooks(v, items),
            "env" => items.push(format!("Environment: {}", keys(v))),
            "permissions" => {
                for (k, v) in v.as_object().into_iter().flatten() {
                    match k.as_str() {
                        "deny" | "ask" => {}
                        "allow" => items.push(format!("Allowed without asking: {}", list(v))),
                        _ => items.push(format!("Permissions {k}: {}", short(v))),
                    }
                }
            }
            _ => items.push(format!("{key}: {}", short(v))),
        }
    }
}

fn hooks(v: &Value, items: &mut Vec<String>) {
    for (event, matchers) in v.as_object().into_iter().flatten() {
        for m in matchers.as_array().into_iter().flatten() {
            for h in m["hooks"].as_array().into_iter().flatten() {
                let what = h["command"]
                    .as_str()
                    .or(h["url"].as_str())
                    .or(h["prompt"].as_str());
                items.push(format!("Hook {event}: {}", what.unwrap_or("?")));
            }
        }
    }
}

fn mcp(text: &str, items: &mut Vec<String>) {
    let Ok(v) = serde_json::from_str::<Value>(text) else {
        items.push(".mcp.json: does not parse, so it can't be listed".into());
        return;
    };
    for (name, s) in v["mcpServers"].as_object().into_iter().flatten() {
        let args = s["args"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str);
        let run: Vec<_> = s["command"].as_str().into_iter().chain(args).collect();
        let what = s["url"].as_str().map(String::from).unwrap_or(run.join(" "));
        items.push(format!("MCP server {name}: {what}"));
    }
}

/// Keys in a skill's, agent's or command's front matter that run or grant something.
fn front(file: &str, text: &str, items: &mut Vec<String>) {
    let mut lines = text.lines();
    if lines.next().map(str::trim) != Some("---") {
        return;
    }
    for line in lines.take_while(|l| l.trim() != "---") {
        if let Some((key, value)) = line.split_once(':')
            && FRONT.contains(&key.trim())
            && !line.starts_with(char::is_whitespace)
        {
            let value = value.trim();
            items.push(match value.is_empty() {
                true => format!("{file}: {}", key.trim()),
                false => format!("{file}: {}: {value}", key.trim()),
            });
        }
    }
}

/// Watches a tree's trust files while it lives.
pub struct Watch(#[allow(dead_code)] Arc<Mutex<RecommendedWatcher>>);

/// Calls `changed` on its own thread whenever a file under `tree` that `globs` (a provider's
/// `trust`) may name changes. Each glob's fixed folder is watched, recursively when the glob goes
/// below it, and that folder's parents, so one made later is seen.
pub fn watch(
    tree: &Path,
    globs: &'static [&'static str],
    changed: impl Fn() + Send + 'static,
) -> Result<Watch> {
    let root = tree.canonicalize()?;
    let mut dirs: BTreeMap<PathBuf, bool> = BTreeMap::new();
    for g in globs {
        let parts: Vec<&str> = g.split('/').collect();
        let fixed = parts.iter().take_while(|p| !p.contains('*')).count();
        let dir: PathBuf = parts[..fixed.min(parts.len() - 1)].iter().collect();
        for up in dir.ancestors().skip(1) {
            dirs.entry(up.into()).or_default();
        }
        *dirs.entry(dir).or_default() |= fixed < parts.len();
    }
    let (tx, rx) = mpsc::channel();
    let fail = |e: notify::Error| Error::Other(e.to_string());
    let watcher = Arc::new(Mutex::new(
        notify::recommended_watcher(move |e| _ = tx.send(e)).map_err(fail)?,
    ));
    let mut watched = HashSet::new();
    let base = root.clone();
    let add = move |w: &mut RecommendedWatcher, watched: &mut HashSet<PathBuf>| {
        for (dir, deep) in &dirs {
            let mode = match deep {
                true => RecursiveMode::Recursive,
                false => RecursiveMode::NonRecursive,
            };
            let path = base.join(dir);
            if !watched.contains(dir) && path.is_dir() && w.watch(&path, mode).is_ok() {
                watched.insert(dir.clone());
            }
        }
    };
    add(&mut watcher.lock().unwrap(), &mut watched);
    let weak = Arc::downgrade(&watcher);
    std::thread::spawn(move || {
        for event in rx.into_iter().flatten() {
            let named = |p: &PathBuf| {
                let rel = p.strip_prefix(&root);
                rel.is_ok_and(|rel| globs.iter().any(|g| names(g, rel)))
            };
            if !event.paths.iter().any(named) {
                continue;
            }
            let Some(w) = weak.upgrade() else { return };
            add(&mut w.lock().unwrap(), &mut watched);
            changed();
        }
    });
    Ok(Watch(watcher))
}

/// Whether `glob` may name `rel` or a file under it, ignoring case where the file system does.
pub fn names(glob: &str, rel: &Path) -> bool {
    let same = |c: &OsStr, p: &str| match cfg!(any(target_os = "macos", windows)) {
        true => c.eq_ignore_ascii_case(p),
        false => c == p,
    };
    let mut parts = glob.split('/');
    for c in rel.components() {
        match parts.next() {
            Some("**") => return true,
            Some(p) if p == "*" || same(c.as_os_str(), p) => {}
            _ => return false,
        }
    }
    rel.parent().is_some()
}

fn keys(v: &Value) -> String {
    let k: Vec<_> = v
        .as_object()
        .into_iter()
        .flatten()
        .map(|(k, _)| k.as_str())
        .collect();
    k.join(", ")
}

fn list(v: &Value) -> String {
    let all: Vec<_> = v
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    all.join(", ")
}

fn short(v: &Value) -> String {
    let s = v.to_string();
    match s.char_indices().nth(120) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::testutil::Sandbox;

    fn write(r: &Path, p: &str, text: &str) {
        fs::create_dir_all(r.join(p).parent().unwrap()).unwrap();
        fs::write(r.join(p), text).unwrap();
    }

    #[test]
    fn lists_what_would_run_and_skips_what_runs_nothing() {
        let sb = Sandbox::new();
        let r = sb.path("r");
        let settings = r#"{"model":"opus","hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"./check.sh"}]}]},
            "env":{"A":"1","B":"2"},"apiKeyHelper":"./key.sh","permissions":{"allow":["Bash(npm *)"],"deny":["Read(.env)"]}}"#;
        write(&r, ".claude/settings.json", settings);
        write(
            &r,
            ".mcp.json",
            r#"{"mcpServers":{"db":{"command":"npx","args":["db-mcp"]}}}"#,
        );
        write(
            &r,
            ".claude/skills/x/SKILL.md",
            "---\nname: x\nallowed-tools: Bash\nhooks:\n  Stop: []\n---\nbody\nhooks: no\n",
        );
        write(&r, ".claude/agents/plain.md", "---\nname: plain\n---\n");
        let mut p = project(&sb.state.env(), &r).unwrap();
        // Key order follows serde_json's features, which the workspace may change.
        p.items.sort();
        let mut want = [
            "apiKeyHelper: \"./key.sh\"",
            "Environment: A, B",
            "Hook PreToolUse: ./check.sh",
            "Allowed without asking: Bash(npm *)",
            ".claude/skills/x/SKILL.md: allowed-tools: Bash",
            ".claude/skills/x/SKILL.md: hooks",
            "MCP server db: npx db-mcp",
        ];
        want.sort();
        assert_eq!(p.items, want);
        assert_eq!(p.fingerprint.lines().count(), 4);
    }

    #[test]
    fn trust_holds_until_a_file_changes_or_appears() {
        let sb = Sandbox::new();
        let r = sb.path("r");
        let (env, config) = (sb.state.env(), sb.path("config"));
        let repo = r.display().to_string();
        assert!(
            trusted(&config, &repo, &project(&env, &r).unwrap()),
            "nothing to trust"
        );
        write(
            &r,
            ".claude/settings.json",
            r#"{"hooks":{"Stop":[{"hooks":[{"command":"a"}]}]}}"#,
        );
        let p = project(&env, &r).unwrap();
        assert!(!trusted(&config, &repo, &p));
        trust(&config, &repo, &p).unwrap();
        assert!(trusted(&config, &repo, &project(&env, &r).unwrap()));
        write(
            &r,
            ".claude/commands/go.md",
            "---\nallowed-tools: Bash\n---\n",
        );
        assert!(!trusted(&config, &repo, &project(&env, &r).unwrap()));
        fs::remove_file(r.join(".claude/commands/go.md")).unwrap();
        write(
            &r,
            ".claude/settings.json",
            r#"{"hooks":{"Stop":[{"hooks":[{"command":"b"}]}]}}"#,
        );
        assert!(!trusted(&config, &repo, &project(&env, &r).unwrap()));
    }

    #[test]
    fn the_watch_sees_a_trust_file_in_a_folder_made_after_it_started() {
        let sb = Sandbox::new();
        let r = sb.path("r");
        fs::create_dir_all(&r).unwrap();
        let (tx, rx) = mpsc::channel();
        let _watch = watch(&r, crate::adapters::CLAUDE.trust, move || _ = tx.send(())).unwrap();
        let wait = || rx.recv_timeout(crate::testutil::scaled(std::time::Duration::from_secs(60)));
        write(&r, ".claude/settings.json", "{}");
        wait().expect("the new .claude folder");
        while rx.try_recv().is_ok() {}
        write(&r, ".claude/commands/go.md", "---\n---\n");
        wait().expect("a file in it");
    }

    #[test]
    fn the_watch_names_trust_files_and_their_folders_alone() {
        let named = |rel: &str| {
            let rel = Path::new(rel);
            crate::adapters::CLAUDE.trust.iter().any(|g| names(g, rel))
        };
        for rel in [
            ".claude",
            ".claude/settings.json",
            ".claude/skills/x",
            ".claude/skills/x/SKILL.md",
            ".claude/commands/a/b.md",
            ".mcp.json",
        ] {
            assert!(named(rel), "{rel}");
        }
        for rel in [
            "",
            ".claude/worktrees/w/target/a.o",
            ".claude/skills/x/y/SKILL.md",
            "src/a.rs",
        ] {
            assert!(!named(rel), "{rel}");
        }
        let folded = named(".Claude/SETTINGS.local.json");
        assert_eq!(folded, cfg!(any(target_os = "macos", windows)));
    }

    #[test]
    fn codex_s_project_layer_and_skills_are_trust_files_on_disk_and_at_a_sha() {
        let sb = Sandbox::new();
        let r = sb.path("r");
        fs::create_dir_all(&r).unwrap();
        sb.git(&r, &["init", "-q"]);
        write(&r, ".codex/config.toml", "  \n");
        write(&r, ".agents/skills/y/SKILL.md", "---\nhooks: x\n---\n");
        let env = sb.state.env();
        let p = project(&env, &r).unwrap();
        assert_eq!(p.items, [".agents/skills/y/SKILL.md: hooks: x"]);
        assert_eq!(p.fingerprint.lines().count(), 2, "an empty file is hashed");
        let repo = r.display().to_string();
        let layer = [
            (".codex/config.toml", "[mcp_servers.x]\ncommand = \"x\"\n"),
            (
                ".codex/rules/x.rules",
                "prefix_rule(pattern = [\"rm\"], decision = \"allow\")\n",
            ),
            (".codex/hooks.json", "{\"hooks\": {}}\n"),
        ];
        for (file, text) in layer {
            trust(&sb.state.config_dir, &repo, &project(&env, &r).unwrap()).unwrap();
            write(&r, file, text);
            let p = project(&env, &r).unwrap();
            assert!(
                p.items.contains(&format!("{file}: unparsed")),
                "{:?}",
                p.items
            );
            assert!(
                !trusted(&sb.state.config_dir, &repo, &p),
                "{file} asks again"
            );
        }
        sb.git(&r, &["add", "-A"]);
        sb.git(&r, &["commit", "-qm", "a"]);
        let head = sb.git(&r, &["rev-parse", "HEAD"]);
        assert_eq!(at(&env, &r, &head).unwrap(), project(&env, &r).unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn a_link_in_or_as_codex_s_layer_or_its_size_refuses_codex_alone() {
        use crate::adapters::{CLAUDE, CODEX};
        let sb = Sandbox::new();
        let r = sb.path("r");
        fs::create_dir_all(r.join(".codex")).unwrap();
        sb.git(&r, &["init", "-q"]);
        write(&r, ".claude/settings.json", r#"{"env": {"A": "1"}}"#);
        let env = sb.state.env();
        let refused = |why: &str| {
            let p = project(&env, &r).unwrap();
            assert_eq!(p.items, ["Environment: A"], "the rest is scanned");
            assert!(p.refuses(CLAUDE.trust).is_none());
            let codex = p.refuses(CODEX.trust).unwrap_or_default().to_string();
            assert!(codex.contains(why), "{codex}");
        };
        std::os::unix::fs::symlink("/dev/zero", r.join(".codex/hooks.json")).unwrap();
        refused("link");
        fs::remove_dir_all(r.join(".codex")).unwrap();
        std::os::unix::fs::symlink("/", r.join(".codex")).unwrap();
        refused("link");
        fs::remove_file(r.join(".codex")).unwrap();
        write(&r, ".codex/config.toml", &"#\n".repeat(1 << 20));
        refused("1 MiB");
        fs::remove_dir_all(r.join(".codex")).unwrap();
        std::os::unix::fs::symlink("../.claude", r.join(".agents")).unwrap();
        sb.git(&r, &["add", "-A"]);
        sb.git(&r, &["commit", "-qm", "a"]);
        let head = sb.git(&r, &["rev-parse", "HEAD"]);
        let p = at(&env, &r, &head).unwrap();
        assert!(p.refuses(CLAUDE.trust).is_none());
        assert!(p.refuses(CODEX.trust).unwrap().contains("link"));
    }

    #[cfg(unix)]
    #[test]
    fn the_scan_at_a_sha_reads_blobs_and_refuses_links_and_size() {
        let sb = crate::git::testutil::Sandbox::new();
        let repo = sb.path("r");
        fs::create_dir_all(repo.join(".claude/skills/x")).unwrap();
        sb.git(&repo, &["init", "-q"]);
        let hook =
            r#"{"hooks": {"PreToolUse": [{"hooks": [{"type": "command", "command": "evil"}]}]}}"#;
        fs::write(repo.join(".claude/settings.json"), hook).unwrap();
        fs::write(
            repo.join(".claude/skills/x/SKILL.md"),
            "---\nname: x\n---\n",
        )
        .unwrap();
        sb.git(&repo, &["add", "-A"]);
        sb.git(&repo, &["commit", "-qm", "a"]);
        let env = sb.state.env();
        let head = sb.git(&repo, &["rev-parse", "HEAD"]);
        let p = at(&env, &repo, &head).unwrap();
        assert_eq!(p, project(&env, &repo).unwrap());
        assert!(!p.items.is_empty());
        fs::remove_file(repo.join(".claude/skills/x/SKILL.md")).unwrap();
        std::os::unix::fs::symlink("/dev/zero", repo.join(".claude/skills/x/SKILL.md")).unwrap();
        sb.git(&repo, &["add", "-A"]);
        sb.git(&repo, &["commit", "-qm", "link"]);
        let head = sb.git(&repo, &["rev-parse", "HEAD"]);
        assert!(
            at(&env, &repo, &head)
                .unwrap_err()
                .to_string()
                .contains("link")
        );
        fs::remove_file(repo.join(".claude/skills/x/SKILL.md")).unwrap();
        fs::remove_dir(repo.join(".claude/skills/x")).unwrap();
        std::os::unix::fs::symlink("/etc", repo.join(".claude/skills/x")).unwrap();
        sb.git(&repo, &["add", "-A"]);
        sb.git(&repo, &["commit", "-qm", "dir link"]);
        let head = sb.git(&repo, &["rev-parse", "HEAD"]);
        assert!(at(&env, &repo, &head).is_err());
        fs::remove_file(repo.join(".claude/skills/x")).unwrap();
        fs::write(repo.join(".mcp.json"), " ".repeat(2 << 20)).unwrap();
        sb.git(&repo, &["add", "-A"]);
        sb.git(&repo, &["commit", "-qm", "big"]);
        let head = sb.git(&repo, &["rev-parse", "HEAD"]);
        assert!(
            at(&env, &repo, &head)
                .unwrap_err()
                .to_string()
                .contains("1 MiB")
        );
    }
}
