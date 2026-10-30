pub mod commands;
pub mod store;
#[cfg(test)]
mod tests;

use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::{Error, Result};
use crate::forge;
use crate::git::runner::Cred;
use crate::state::AppState;
use crate::types::{Account, AccountInput, HostAccount, VerifyResult};

const FILE: &str = "accounts.json";

/// Serializes read-modify-write of `accounts.json`.
static FILE_LOCK: Mutex<()> = Mutex::new(());

pub fn load(dir: &Path) -> Result<Vec<Account>> {
    match std::fs::read(dir.join(FILE)) {
        Ok(b) => Ok(serde_json::from_slice(&b)?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(vec![]),
        Err(e) => Err(e.into()),
    }
}

fn store(dir: &Path, accounts: &[Account]) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!("{FILE}.tmp"));
    std::fs::write(&tmp, serde_json::to_vec_pretty(accounts)?)?;
    std::fs::rename(tmp, dir.join(FILE))?;
    Ok(())
}

fn new_id() -> String {
    static N: AtomicU32 = AtomicU32::new(0);
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format!(
        "acc-{:x}-{}",
        t.as_nanos(),
        N.fetch_add(1, Ordering::Relaxed)
    )
}

/// Creates or updates an account; a `None` token keeps the stored one.
pub fn save(state: &AppState, input: AccountInput) -> Result<Account> {
    let base_url = input.base_url.trim().trim_end_matches('/').to_string();
    if https(&base_url).is_none() {
        return Err(Error::Other("Base URL must start with https://".into()));
    }
    let _lock = FILE_LOCK.lock().unwrap();
    let mut accounts = load(&state.config_dir)?;
    if let Some(id) = &input.id
        && !accounts.iter().any(|a| a.id == *id)
    {
        return Err(Error::Other(format!("unknown account {id}")));
    }
    let token = input
        .token
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty());
    let account = Account {
        id: input.id.unwrap_or_else(new_id),
        provider: input.provider,
        base_url,
        username: input.username.trim().to_string(),
        label: input.label.trim().to_string(),
    };
    if let Some(token) = token {
        state.tokens.set(&account.id, token)?;
    }
    match accounts.iter_mut().find(|a| a.id == account.id) {
        Some(a) => *a = account.clone(),
        None => accounts.push(account.clone()),
    }
    store(&state.config_dir, &accounts)?;
    Ok(account)
}

/// Deletes the account, its token and every repo override pointing at it.
pub fn remove(state: &AppState, id: &str) -> Result<()> {
    state.tokens.delete(id)?;
    {
        let _lock = FILE_LOCK.lock().unwrap();
        let mut accounts = load(&state.config_dir)?;
        accounts.retain(|a| a.id != id);
        store(&state.config_dir, &accounts)?;
    }
    let mut s = state.settings.lock().unwrap();
    s.repo_accounts.retain(|_, a| a != id);
    s.forge_accounts.retain(|_, hosts| {
        hosts.retain(|_, a| a != id);
        !hosts.is_empty()
    });
    s.save(&state.config_dir)
}

pub fn verify(state: &AppState, id: &str) -> Result<VerifyResult> {
    let account = load(&state.config_dir)?
        .into_iter()
        .find(|a| a.id == id)
        .ok_or_else(|| Error::Other(format!("unknown account {id}")))?;
    Ok(match state.tokens.get(id)? {
        Some(token) => forge::verify(&account.provider, &account.base_url, &token),
        None => VerifyResult::Failed {
            status: None,
            error: "no token stored".into(),
        },
    })
}

/// Remote names grouped by https host, with the account each host resolves to.
pub fn repo_accounts(state: &AppState, repo: &Path) -> Result<Vec<HostAccount>> {
    // Same URLs as `credentials` gets: `insteadOf` applied, push URLs included.
    let remotes = crate::git::remotes::remotes(&state.env(), repo)?;
    let accounts = load(&state.config_dir)?;
    let over = repo_override(state, repo);
    let mut hosts: Vec<HostAccount> = vec![];
    for (name, url) in &remotes {
        let Some((host, account, ambiguous)) = resolve(&accounts, over.as_deref(), url) else {
            continue;
        };
        match hosts.iter_mut().find(|h| h.host == host) {
            Some(h) if h.remotes.contains(name) => {}
            Some(h) => h.remotes.push(name.clone()),
            None => hosts.push(HostAccount {
                host,
                remotes: vec![name.clone()],
                account_id: account.map(|a| a.id.clone()),
                ambiguous,
            }),
        }
    }
    Ok(hosts)
}

/// Credential arguments for the https remotes in `remote_urls` that have a matched account.
pub fn credentials(state: &AppState, repo: &Path, remote_urls: &[String]) -> Result<Option<Cred>> {
    let accounts = load(&state.config_dir)?;
    let over = repo_override(state, repo);
    let mut seen = vec![];
    let mut cred = Cred::default();
    for (host, account, _) in remote_urls
        .iter()
        .filter_map(|u| resolve(&accounts, over.as_deref(), u))
    {
        if seen.contains(&host) {
            continue;
        }
        seen.push(host.clone());
        let Some(account) = account else { continue };
        let Some(token) = state.tokens.get(&account.id)? else {
            continue;
        };
        let n = cred.env.len() / 2;
        let key = format!("credential.https://{host}.helper");
        cred.args.extend([
            "-c".into(),
            format!("{key}="),
            "-c".into(),
            format!(
                r#"{key}=!f() {{ test "$1" = get && printf "username=%s\npassword=%s\n" "$WB_USER_{n}" "$WB_TOKEN_{n}"; }}; f"#
            ),
        ]);
        cred.env
            .push((format!("WB_USER_{n}"), account.username.clone()));
        cred.env.push((format!("WB_TOKEN_{n}"), token));
    }
    Ok((!cred.args.is_empty()).then_some(cred))
}

fn repo_override(state: &AppState, repo: &Path) -> Option<String> {
    let s = state.settings.lock().unwrap();
    s.repo_accounts.get(repo.to_str()?).cloned()
}

/// A remote URL's host and path (`/group/app.git`). The host is lowercased and keeps its port for
/// https; for ssh it is the bare host name, before any `ssh -G` resolution.
#[derive(Debug, Clone, PartialEq)]
pub struct RemoteUrl {
    pub ssh: bool,
    pub host: String,
    pub path: String,
}

/// Parses https, `ssh://` and scp-style (`git@host:group/app.git`) URLs; None for anything else.
pub fn parse_remote(url: &str) -> Option<RemoteUrl> {
    let (ssh, authority, path) = if let Some(rest) = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://").filter(|_| test_http(url)))
    {
        let (a, p) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
        (false, a, p.to_string())
    } else if let Some(rest) = url.strip_prefix("ssh://") {
        let (a, p) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
        (true, a, p.to_string())
    } else {
        // scp-style needs a colon before any slash, else it is a local path.
        let (a, p) = url.split_once(':')?;
        if a.contains('/') || url.contains("://") {
            return None;
        }
        (true, a, format!("/{p}"))
    };
    let mut host = authority.rsplit('@').next()?.to_ascii_lowercase();
    if ssh && let Some((h, _port)) = host.rsplit_once(':') {
        host = h.to_string();
    }
    (!host.is_empty()).then(|| RemoteUrl {
        ssh,
        host,
        path: path.trim_end_matches('/').to_string(),
    })
}

/// `http://127.0.0.1…` or `http://localhost…`: taken as https only in tests and e2e, for local fakes.
pub fn test_http(url: &str) -> bool {
    cfg!(any(test, feature = "e2e"))
        && url
            .strip_prefix("http://")
            .and_then(|r| r.split([':', '/']).next())
            .is_some_and(|h| h == "127.0.0.1" || h == "localhost")
}

/// An account fits an https remote on host and base-path prefix, an ssh remote on host name alone.
pub fn fits(account: &Account, remote: &RemoteUrl) -> bool {
    let Some(base) = parse_remote(&account.base_url).filter(|b| !b.ssh) else {
        return false;
    };
    if remote.ssh {
        return base.host.split(':').next() == Some(remote.host.as_str());
    }
    base.host == remote.host
        && remote
            .path
            .strip_prefix(&base.path)
            .is_some_and(|r| r.is_empty() || r.starts_with('/'))
}

/// `(host[:port], path)` of an https URL; `None` for ssh, scp-like and other schemes.
fn https(url: &str) -> Option<(String, String)> {
    parse_remote(url)
        .filter(|r| !r.ssh)
        .map(|r| (r.host, r.path))
}

/// The https host of `url`, its account (override first, else the only fitting one) and
/// whether several accounts fit with no override.
fn resolve<'a>(
    accounts: &'a [Account],
    over: Option<&str>,
    url: &str,
) -> Option<(String, Option<&'a Account>, bool)> {
    let remote = parse_remote(url).filter(|r| !r.ssh)?;
    let fits: Vec<&Account> = accounts.iter().filter(|a| fits(a, &remote)).collect();
    let account = match fits.iter().find(|a| Some(a.id.as_str()) == over) {
        Some(a) => Some(*a),
        None if fits.len() == 1 => Some(fits[0]),
        None => None,
    };
    Some((remote.host, account, account.is_none() && fits.len() > 1))
}
