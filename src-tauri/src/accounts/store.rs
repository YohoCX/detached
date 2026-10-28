use std::collections::HashMap;
use std::sync::Mutex;

use crate::error::Result;

pub trait TokenStore: Send + Sync {
    fn get(&self, id: &str) -> Result<Option<String>>;
    fn set(&self, id: &str, token: &str) -> Result<()>;
    fn delete(&self, id: &str) -> Result<()>;
}

/// For tests, and the scaffold until agent B adds the Keychain store.
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
