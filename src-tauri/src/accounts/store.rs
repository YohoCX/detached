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

/// macOS Keychain generic passwords: service `workbench`, account = account id.
pub struct KeychainStore;

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
        let item = generic_password(PasswordOptions::new_generic_password(SERVICE, id));
        Ok(keychain(item)?.map(|b| String::from_utf8_lossy(&b).into_owned()))
    }

    fn set(&self, id: &str, token: &str) -> Result<()> {
        keychain(set_generic_password(SERVICE, id, token.as_bytes())).map(drop)
    }

    fn delete(&self, id: &str) -> Result<()> {
        keychain(delete_generic_password(SERVICE, id)).map(drop)
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
