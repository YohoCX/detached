use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use crate::accounts::store::TokenStore;
use crate::env::Env;
use crate::settings::Settings;

pub struct AppState {
    pub config_dir: PathBuf,
    pub settings: Mutex<Settings>,
    pub env: RwLock<Env>,
    pub ops: Ops,
    pub locks: RepoLocks,
    pub tokens: Arc<dyn TokenStore>,
}

impl AppState {
    pub fn env(&self) -> Env {
        self.env.read().unwrap().clone()
    }
}

/// Cancellation flag for one running operation.
pub type Cancel = Arc<AtomicBool>;

/// Running cancellable operations by the UI's op id.
#[derive(Default)]
pub struct Ops(Mutex<HashMap<u64, Cancel>>);

impl Ops {
    pub fn start(&self, id: u64) -> Cancel {
        let c = Cancel::default();
        self.0.lock().unwrap().insert(id, c.clone());
        c
    }

    pub fn cancel(&self, id: u64) {
        if let Some(c) = self.0.lock().unwrap().get(&id) {
            c.store(true, Ordering::SeqCst);
        }
    }

    pub fn finish(&self, id: u64) {
        self.0.lock().unwrap().remove(&id);
    }
}

/// One mutex per repo, serializing mutating git operations.
#[derive(Default)]
pub struct RepoLocks(Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>);

impl RepoLocks {
    pub fn get(&self, repo: &Path) -> Arc<Mutex<()>> {
        self.0
            .lock()
            .unwrap()
            .entry(repo.to_path_buf())
            .or_default()
            .clone()
    }
}
