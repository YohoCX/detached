pub mod commands;
pub mod store;
#[cfg(test)]
mod tests;

use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::{Error, Result};
use crate::git::runner::Cred;
use crate::providers;
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
    s.save(&state.config_dir)
}

pub fn verify(state: &AppState, id: &str) -> Result<VerifyResult> {
    let account = load(&state.config_dir)?
        .into_iter()
        .find(|a| a.id == id)
        .ok_or_else(|| Error::Other(format!("unknown account {id}")))?;
    Ok(match state.tokens.get(id)? {
        Some(token) => providers::verify(&account.provider, &account.base_url, &token),
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

/// `(host[:port], path)` of an https URL; `None` for ssh, scp-like and other schemes.
fn https(url: &str) -> Option<(String, String)> {
    let rest = url.strip_prefix("https://")?;
    let (authority, path) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
    let host = authority.rsplit('@').next()?.to_ascii_lowercase();
    (!host.is_empty()).then(|| (host, path.trim_end_matches('/').to_string()))
}

/// The https host of `url`, its account (override first, else the only fitting one) and
/// whether several accounts fit with no override.
fn resolve<'a>(
    accounts: &'a [Account],
    over: Option<&str>,
    url: &str,
) -> Option<(String, Option<&'a Account>, bool)> {
    let (host, path) = https(url)?;
    let fits: Vec<&Account> = accounts
        .iter()
        .filter(|a| {
            https(&a.base_url).is_some_and(|(h, prefix)| {
                h == host
                    && path
                        .strip_prefix(&prefix)
                        .is_some_and(|r| r.is_empty() || r.starts_with('/'))
            })
        })
        .collect();
    let account = match fits.iter().find(|a| Some(a.id.as_str()) == over) {
        Some(a) => Some(*a),
        None if fits.len() == 1 => Some(fits[0]),
        None => None,
    };
    Some((host, account, account.is_none() && fits.len() > 1))
}
