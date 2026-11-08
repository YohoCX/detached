//! Which repos may load their own Claude settings: what those would run, and the user's yes to it.
//!
//! An SDK session shows no trust dialog, so a repo's hooks would run unasked.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::env::Env;
use crate::error::Result;
use crate::git::runner::{Kind, out};

const FILE: &str = "agent-trust.json";
const SETTINGS: &[&str] = &[".claude/settings.json", ".claude/settings.local.json"];
const MARKDOWN: &[&str] = &[".claude/skills", ".claude/agents", ".claude/commands"];
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
    files.sort();
    let mut items = vec![];
    for f in &files {
        let text = fs::read_to_string(repo.join(f)).unwrap_or_default();
        match f.as_str() {
            ".mcp.json" => mcp(&text, &mut items),
            f if f.ends_with(".json") => settings(f, &text, &mut items),
            f => front(f, &text, &mut items),
        }
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
    Ok(Project { items, fingerprint })
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
}
