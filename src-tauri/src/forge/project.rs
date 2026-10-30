use std::path::Path;
use std::process::{Command, Stdio};

use crate::accounts::{self, RemoteUrl};
use crate::env::Env;
use crate::error::{Error, Result};
use crate::git::runner::{Kind, Opts, out, run};
use crate::settings::Settings;
use crate::state::AppState;
use crate::types::{Account, ForgeHosts, HostAccount, Project};

/// A remote whose URL names a forge project, with the account its API calls use.
#[derive(Debug, Clone)]
pub struct Remote {
    pub name: String,
    /// Host after `ssh -G`.
    pub url: RemoteUrl,
    pub account: Option<Account>,
    /// Several accounts fit and none is chosen.
    pub ambiguous: bool,
}

impl Remote {
    /// The project on the account's host, the base URL's sub-path stripped.
    pub fn project(&self) -> Result<Project> {
        let host = &self.url.host;
        let a = match (&self.account, self.ambiguous) {
            (Some(a), _) => a,
            (None, true) => {
                return Err(Error::Other(format!(
                    "Several accounts fit {host}; choose one in Settings"
                )));
            }
            (None, false) => {
                return Err(Error::Other(format!(
                    "No account for {host}; add one in Settings"
                )));
            }
        };
        let base = accounts::parse_remote(&a.base_url)
            .ok_or_else(|| Error::Other(format!("bad base URL {}", a.base_url)))?;
        let mut path = self.url.path.as_str();
        if !self.url.ssh {
            path = path.strip_prefix(&base.path).unwrap_or(path);
        }
        let path = path.trim_matches('/');
        Ok(Project {
            host: base.host,
            path: path.strip_suffix(".git").unwrap_or(path).into(),
        })
    }
}

/// The real host of an ssh alias, from `ssh -G`.
fn ssh_host(env: &Env, host: &str) -> Option<String> {
    let ssh = crate::env::which("ssh", &env.path)?;
    let o = Command::new(ssh)
        .args(["-G", "--", host])
        .env("PATH", &env.path)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&o.stdout);
    let h = text.lines().find_map(|l| l.strip_prefix("hostname "))?;
    Some(h.trim().to_ascii_lowercase())
}

/// A remote URL as the forge sees it: ssh aliases resolved, `ssh.github.com` as `github.com`.
pub fn resolve_url(env: &Env, url: &str) -> Option<RemoteUrl> {
    let mut u = accounts::parse_remote(url)?;
    if u.ssh
        && let Some(h) = ssh_host(env, &u.host)
    {
        u.host = h;
    }
    if u.host == "ssh.github.com" {
        u.host = "github.com".into();
    }
    Some(u)
}

/// The repo's forge account for the host, else the only fitting one, else milestone 1's choice
/// when it fits; with none, whether several fit.
pub fn account(
    s: &Settings,
    accounts: &[Account],
    repo: &str,
    url: &RemoteUrl,
) -> (Option<Account>, bool) {
    let chosen = s.forge_accounts.get(repo).and_then(|h| h.get(&url.host));
    if let Some(a) = chosen.and_then(|id| accounts.iter().find(|a| a.id == *id)) {
        return (Some(a.clone()), false);
    }
    let fit: Vec<&Account> = accounts.iter().filter(|a| accounts::fits(a, url)).collect();
    let m1 = s.repo_accounts.get(repo);
    match fit.iter().find(|a| fit.len() == 1 || Some(&a.id) == m1) {
        Some(a) => (Some((*a).clone()), false),
        None => (None, fit.len() > 1),
    }
}

/// The repo's remotes that name a forge project, by their fetch URL.
pub fn remotes(state: &AppState, repo: &Path) -> Result<Vec<Remote>> {
    let env = state.env();
    let accounts = accounts::load(&state.config_dir)?;
    let settings = state.get_settings();
    let key = repo.to_str().unwrap_or_default();
    let mut v: Vec<Remote> = vec![];
    let mut seen = vec![];
    for (name, url) in crate::git::remotes::remotes(&env, repo)? {
        if seen.contains(&name) {
            continue;
        }
        seen.push(name.clone());
        if let Some(url) = resolve_url(&env, &url) {
            let (account, ambiguous) = account(&settings, &accounts, key, &url);
            v.push(Remote {
                name,
                url,
                account,
                ambiguous,
            });
        }
    }
    Ok(v)
}

/// The saved MR remote while it exists, else `origin`, else the only remote.
pub fn mr_remote(s: &Settings, repo: &str, names: &[&str]) -> Option<String> {
    let saved = s.mr_remotes.get(repo).map(String::as_str);
    saved
        .filter(|r| names.contains(r))
        .or(names.contains(&"origin").then_some("origin"))
        .or((names.len() == 1).then(|| names[0]))
        .map(String::from)
}

pub fn hosts(state: &AppState, repo: &Path) -> Result<ForgeHosts> {
    let names = crate::git::remotes::remotes(&state.env(), repo)?;
    let mut names: Vec<&str> = names.iter().map(|(n, _)| n.as_str()).collect();
    names.dedup();
    let mut hosts: Vec<HostAccount> = vec![];
    for r in remotes(state, repo)? {
        match hosts.iter_mut().find(|h| h.host == r.url.host) {
            Some(h) => h.remotes.push(r.name),
            None => hosts.push(HostAccount {
                host: r.url.host,
                remotes: vec![r.name],
                account_id: r.account.map(|a| a.id),
                ambiguous: r.ambiguous,
            }),
        }
    }
    Ok(ForgeHosts {
        hosts,
        mr_remote: mr_remote(
            &state.get_settings(),
            repo.to_str().unwrap_or_default(),
            &names,
        ),
    })
}

/// The remote holding the repo's MRs.
pub fn mr(state: &AppState, repo: &Path) -> Result<Remote> {
    let all = crate::git::remotes::remotes(&state.env(), repo)?;
    let mut names: Vec<&str> = all.iter().map(|(n, _)| n.as_str()).collect();
    names.dedup();
    let key = repo.to_str().unwrap_or_default();
    let name = mr_remote(&state.get_settings(), key, &names)
        .ok_or_else(|| Error::Other("No MR remote; choose one in Settings".into()))?;
    remotes(state, repo)?
        .into_iter()
        .find(|r| r.name == name)
        .ok_or_else(|| Error::Other(format!("Remote {name} is not on a forge")))
}

/// The current branch on its forge: the remote and branch of `@{push}`, else of `@{upstream}`.
#[derive(Debug, Clone, PartialEq)]
pub struct Source {
    pub remote: String,
    pub branch: String,
    /// The remote-tracking ref's SHA, when it exists.
    pub sha: Option<String>,
}

pub fn source(env: &Env, repo: &Path) -> Result<Option<Source>> {
    let head = run(
        env,
        repo,
        &["symbolic-ref", "-q", "HEAD"],
        Kind::Read,
        Opts::default(),
    )?;
    let head = head.text();
    if head.trim().is_empty() {
        return Ok(None);
    }
    let fmt = "--format=%(push:remotename)%00%(push)%00%(upstream:remotename)%00%(upstream)";
    let line = out(env, repo, &["for-each-ref", fmt, head.trim()], Kind::Read)?;
    let f: Vec<&str> = line.trim_end().split('\0').collect();
    let pick = |remote: &str, tracking: &str| {
        let branch = tracking.strip_prefix(&format!("refs/remotes/{remote}/"))?;
        Some((remote.to_string(), branch.to_string(), tracking.to_string()))
    };
    let Some((remote, branch, tracking)) = (f.len() == 4)
        .then(|| pick(f[0], f[1]).or_else(|| pick(f[2], f[3])))
        .flatten()
    else {
        return Ok(None);
    };
    let sha = run(
        env,
        repo,
        &[
            "rev-parse",
            "-q",
            "--verify",
            &format!("{tracking}^{{commit}}"),
        ],
        Kind::Read,
        Opts::default(),
    )?;
    let sha = Some(sha.text().trim().to_string()).filter(|s| !s.is_empty());
    Ok(Some(Source {
        remote,
        branch,
        sha,
    }))
}
