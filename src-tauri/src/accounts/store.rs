use std::collections::HashMap;
use std::sync::Mutex;

use security_framework::passwords::{
    PasswordOptions, delete_generic_password, generic_password, set_generic_password,
};

use crate::error::{Error, Result};

pub trait TokenStore: Send + Sync {
    fn get(&self, id: &str) -> Result<Option<String>>;
    fn set(&self, id: &str, token: &str) -> Result<()>;
    fn delete(&self, id: &str) -> Result<()>;
}

/// macOS Keychain generic passwords: service `workbench` (the app's former name, kept so saved tokens
/// still resolve), account = account id. Reads are cached for the session: each Keychain read may
/// ask the user, and fetch all would otherwise read the same token once per repo.
#[derive(Default)]
pub struct KeychainStore(Mutex<HashMap<String, Option<String>>>);

const SERVICE: &str = "workbench";
const ITEM_NOT_FOUND: i32 = -25300;

fn keychain<T>(r: std::result::Result<T, security_framework::base::Error>) -> Result<Option<T>> {
    match r {
        Ok(v) => Ok(Some(v)),
        Err(e) if e.code() == ITEM_NOT_FOUND => Ok(None),
        Err(e) => Err(Error::Other(format!("Keychain: {e}"))),
    }
}

impl TokenStore for KeychainStore {
    fn get(&self, id: &str) -> Result<Option<String>> {
        let mut cache = self.0.lock().unwrap();
        if let Some(t) = cache.get(id) {
            return Ok(t.clone());
        }
        let item = generic_password(PasswordOptions::new_generic_password(SERVICE, id));
        let token = keychain(item)?.map(|b| String::from_utf8_lossy(&b).into_owned());
        cache.insert(id.into(), token.clone());
        Ok(token)
    }

    fn set(&self, id: &str, token: &str) -> Result<()> {
        keychain(set_generic_password(SERVICE, id, token.as_bytes()))?;
        self.0.lock().unwrap().insert(id.into(), Some(token.into()));
        Ok(())
    }

    fn delete(&self, id: &str) -> Result<()> {
        keychain(delete_generic_password(SERVICE, id))?;
        self.0.lock().unwrap().insert(id.into(), None);
        Ok(())
    }
}

/// For tests.
#[derive(Default)]
pub struct MemoryStore(Mutex<HashMap<String, String>>);

impl TokenStore for MemoryStore {
    fn get(&self, id: &str) -> Result<Option<String>> {
        Ok(self.0.lock().unwrap().get(id).cloned())
    }

    fn set(&self, id: &str, token: &str) -> Result<()> {
        self.0.lock().unwrap().insert(id.into(), token.into());
        Ok(())
    }

    fn delete(&self, id: &str) -> Result<()> {
        self.0.lock().unwrap().remove(id);
        Ok(())
    }
}
