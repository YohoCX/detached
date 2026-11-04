use std::collections::HashMap;
use std::sync::Mutex;

use crate::error::Result;

pub trait TokenStore: Send + Sync {
    fn get(&self, id: &str) -> Result<Option<String>>;
    fn set(&self, id: &str, token: &str) -> Result<()>;
    fn delete(&self, id: &str) -> Result<()>;
}

/// The system's credential store by account id: Keychain, Credential Manager or Secret Service.
/// Reads are cached for the session: each may prompt the user, and fetch-all would read the same
/// token once per repo.
#[derive(Default)]
pub struct SystemStore(Mutex<HashMap<String, Option<String>>>);

/// The app's former name, kept so saved tokens still resolve.
const SERVICE: &str = "workbench";

impl TokenStore for SystemStore {
    fn get(&self, id: &str) -> Result<Option<String>> {
        let mut cache = self.0.lock().unwrap();
        if let Some(t) = cache.get(id) {
            return Ok(t.clone());
        }
        let token = native::get(id)?;
        cache.insert(id.into(), token.clone());
        Ok(token)
    }

    fn set(&self, id: &str, token: &str) -> Result<()> {
        native::set(id, token)?;
        self.0.lock().unwrap().insert(id.into(), Some(token.into()));
        Ok(())
    }

    fn delete(&self, id: &str) -> Result<()> {
        native::delete(id)?;
        self.0.lock().unwrap().insert(id.into(), None);
        Ok(())
    }
}

#[cfg(target_os = "macos")]
mod native {
    use security_framework::passwords::{
        PasswordOptions, delete_generic_password, generic_password, set_generic_password,
    };

    use super::SERVICE;
    use crate::error::{Error, Result};

    const ITEM_NOT_FOUND: i32 = -25300;

    fn keychain<T>(
        r: std::result::Result<T, security_framework::base::Error>,
    ) -> Result<Option<T>> {
        match r {
            Ok(v) => Ok(Some(v)),
            Err(e) if e.code() == ITEM_NOT_FOUND => Ok(None),
            Err(e) => Err(Error::Other(format!("Keychain: {e}"))),
        }
    }

    pub fn get(id: &str) -> Result<Option<String>> {
        let item = generic_password(PasswordOptions::new_generic_password(SERVICE, id));
        Ok(keychain(item)?.map(|b| String::from_utf8_lossy(&b).into_owned()))
    }

    pub fn set(id: &str, token: &str) -> Result<()> {
        keychain(set_generic_password(SERVICE, id, token.as_bytes())).map(drop)
    }

    pub fn delete(id: &str) -> Result<()> {
        keychain(delete_generic_password(SERVICE, id)).map(drop)
    }
}

#[cfg(not(target_os = "macos"))]
mod native {
    use keyring::{Entry, Error as K};

    use super::SERVICE;
    use crate::error::{Error, Result};

    fn fail(e: K) -> Error {
        Error::Other(format!("Credential store: {e}"))
    }

    fn entry(id: &str) -> Result<Entry> {
        Entry::new(SERVICE, id).map_err(fail)
    }

    pub fn get(id: &str) -> Result<Option<String>> {
        match entry(id)?.get_password() {
            Ok(t) => Ok(Some(t)),
            Err(K::NoEntry) => Ok(None),
            Err(e) => Err(fail(e)),
        }
    }

    pub fn set(id: &str, token: &str) -> Result<()> {
        entry(id)?.set_password(token).map_err(fail)
    }

    pub fn delete(id: &str) -> Result<()> {
        match entry(id)?.delete_credential() {
            Ok(()) | Err(K::NoEntry) => Ok(()),
            Err(e) => Err(fail(e)),
        }
    }
}

/// For tests and e2e.
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
