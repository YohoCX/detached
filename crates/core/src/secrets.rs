//! The env store: each profile's variable names in Settings, their values encrypted in
//! `data/secrets` under a key kept in the system's credential store.

use std::collections::BTreeMap;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use ring::aead::{Aad, CHACHA20_POLY1305, LessSafeKey, NONCE_LEN, Nonce, UnboundKey};
use ring::rand::{SecureRandom, SystemRandom};

use crate::error::{Error, Result};
use crate::state::AppState;

/// Settings' block: profile -> its variables' names.
pub const BLOCK: &str = "envProfiles";
const KEY: &str = "env-store-key";
const FILE: &str = "secrets";

type Values = BTreeMap<String, BTreeMap<String, String>>;

/// Each profile's variable names.
pub fn profiles(state: &AppState) -> BTreeMap<String, Vec<String>> {
    state.get_settings().block(BLOCK).unwrap_or_default()
}

/// `profile`'s variables with their values, by name.
pub fn values(state: &AppState, profile: &str) -> Result<Vec<(String, String)>> {
    let names = profiles(state).remove(profile);
    let names = names.ok_or_else(|| Error::Other(format!("No env profile \"{profile}\".")))?;
    let mut all = read(state)?;
    let mut values = all.remove(profile).unwrap_or_default();
    let take = |n: String| values.remove(&n).map(|v| (n, v));
    Ok(names.into_iter().filter_map(take).collect())
}

/// Sets `profile` to `vars`; an empty value keeps the one stored under that name.
pub fn save(state: &AppState, profile: &str, vars: &[(String, String)]) -> Result<()> {
    if profile.is_empty()
        || !profile
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
    {
        return Err(Error::Other(format!(
            "\"{profile}\" is not a profile name: letters, digits, '.', '_' or '-'."
        )));
    }
    for (name, _) in vars {
        let first = name.chars().next().is_some_and(|c| !c.is_ascii_digit());
        if !first || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err(Error::Other(format!("\"{name}\" is not a variable name.")));
        }
    }
    let path = state.data_dir.join(FILE);
    let lock = state.locks.get(&path);
    let _guard = lock.lock().unwrap();
    let mut all = read(state)?;
    let old = all.remove(profile).unwrap_or_default();
    let kept = |(n, v): &(String, String)| match v.is_empty() {
        true => old.get(n).map(|v| (n.clone(), v.clone())),
        false => Some((n.clone(), v.clone())),
    };
    let values: BTreeMap<_, _> = vars.iter().filter_map(kept).collect();
    if let Some((n, _)) = vars.iter().find(|(n, _)| !values.contains_key(n)) {
        return Err(Error::Other(format!("{n} needs a value.")));
    }
    let names: Vec<String> = values.keys().cloned().collect();
    all.insert(profile.into(), values);
    write(state, &all)?;
    let mut profiles = profiles(state);
    profiles.insert(profile.into(), names);
    state.set_block(BLOCK, profiles)?;
    Ok(())
}

pub fn remove(state: &AppState, profile: &str) -> Result<()> {
    let path = state.data_dir.join(FILE);
    let lock = state.locks.get(&path);
    let _guard = lock.lock().unwrap();
    let mut all = read(state)?;
    if all.remove(profile).is_some() {
        write(state, &all)?;
    }
    let mut profiles = profiles(state);
    profiles.remove(profile);
    state.set_block(BLOCK, profiles)?;
    Ok(())
}

fn cipher(state: &AppState, create: bool) -> Result<Option<LessSafeKey>> {
    let bytes = match state.tokens.get(KEY)? {
        Some(k) => STANDARD
            .decode(k)
            .map_err(|e| Error::Other(e.to_string()))?,
        None if !create => return Ok(None),
        None => {
            let mut k = [0; 32];
            SystemRandom::new().fill(&mut k).map_err(|_| failed())?;
            state.tokens.set(KEY, &STANDARD.encode(k))?;
            k.to_vec()
        }
    };
    let key = UnboundKey::new(&CHACHA20_POLY1305, &bytes).map_err(|_| failed())?;
    Ok(Some(LessSafeKey::new(key)))
}

fn failed() -> Error {
    Error::Other("The env store cannot be read: its key does not open it.".into())
}

fn read(state: &AppState) -> Result<Values> {
    let data = match std::fs::read(state.data_dir.join(FILE)) {
        Ok(d) => d,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Values::new()),
        Err(e) => return Err(e.into()),
    };
    let Some(key) = cipher(state, false)? else {
        return Err(Error::Other(
            "The env store's key is gone from the system's credential store.".into(),
        ));
    };
    if data.len() < NONCE_LEN {
        return Err(failed());
    }
    let (nonce, sealed) = data.split_at(NONCE_LEN);
    let nonce = Nonce::try_assume_unique_for_key(nonce).map_err(|_| failed())?;
    let mut sealed = sealed.to_vec();
    let plain = (key.open_in_place(nonce, Aad::empty(), &mut sealed)).map_err(|_| failed())?;
    Ok(serde_json::from_slice(plain)?)
}

fn write(state: &AppState, all: &Values) -> Result<()> {
    let key = cipher(state, true)?.ok_or_else(failed)?;
    let mut nonce = [0; NONCE_LEN];
    SystemRandom::new().fill(&mut nonce).map_err(|_| failed())?;
    let mut sealed = serde_json::to_vec(all)?;
    let n = Nonce::assume_unique_for_key(nonce);
    (key.seal_in_place_append_tag(n, Aad::empty(), &mut sealed)).map_err(|_| failed())?;
    std::fs::create_dir_all(&state.data_dir)?;
    crate::store::atomic(&state.data_dir.join(FILE), &[&nonce[..], &sealed].concat())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::testutil::Sandbox;

    fn vars(v: &[(&str, &str)]) -> Vec<(String, String)> {
        v.iter()
            .map(|(n, v)| (n.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn values_stay_out_of_settings_and_the_file_and_an_empty_one_keeps_the_old() {
        let sb = Sandbox::new();
        let secret = "not-a-real-secret-4711";
        save(
            &sb.state,
            "dev",
            &vars(&[("API_TOKEN", secret), ("PORT", "3000")]),
        )
        .unwrap();
        assert_eq!(
            values(&sb.state, "dev").unwrap(),
            vars(&[("API_TOKEN", secret), ("PORT", "3000")])
        );
        let settings = std::fs::read_to_string(sb.state.config_dir.join("settings.json")).unwrap();
        assert!(
            settings.contains("API_TOKEN") && !settings.contains(secret),
            "{settings}"
        );
        let file = std::fs::read(sb.state.data_dir.join(FILE)).unwrap();
        assert!(!String::from_utf8_lossy(&file).contains(secret));
        save(&sb.state, "dev", &vars(&[("API_TOKEN", ""), ("HOST", "h")])).unwrap();
        assert_eq!(
            values(&sb.state, "dev").unwrap(),
            vars(&[("API_TOKEN", secret), ("HOST", "h")])
        );
        assert!(save(&sb.state, "dev", &vars(&[("NEW", "")])).is_err());
        assert!(save(&sb.state, "dev", &vars(&[("1X", "v")])).is_err());
        assert!(save(&sb.state, "a b", &vars(&[])).is_err());
        remove(&sb.state, "dev").unwrap();
        assert!(values(&sb.state, "dev").is_err());
        assert!(profiles(&sb.state).is_empty());
    }

    #[test]
    fn without_its_key_the_store_says_so() {
        let sb = Sandbox::new();
        save(&sb.state, "dev", &vars(&[("A", "b")])).unwrap();
        sb.state.tokens.delete(KEY).unwrap();
        let e = values(&sb.state, "dev").unwrap_err().to_string();
        assert!(e.contains("key is gone"), "{e}");
    }
}
