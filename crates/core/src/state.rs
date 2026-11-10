use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use crate::accounts::store::{SystemStore, TokenStore};
use crate::env::Env;
use crate::error::Result;
use crate::settings::Settings;
use crate::{forge, paths, search::quick};

pub struct AppState {
    pub config_dir: PathBuf,
    pub(crate) settings: Mutex<Settings>,
    pub(crate) revision: AtomicU64,
    pub env: RwLock<Env>,
    pub ops: Ops,
    pub locks: RepoLocks,
    pub tokens: Arc<dyn TokenStore>,
    pub(crate) quick: quick::Cache,
    pub(crate) forge: Arc<forge::Cache>,
    /// Serializes read-modify-write of `accounts.json`.
    pub(crate) accounts_lock: Mutex<()>,
}

impl AppState {
    pub fn new(
        config_dir: PathBuf,
        settings: Settings,
        env: Env,
        tokens: Arc<dyn TokenStore>,
    ) -> AppState {
        AppState {
            config_dir,
            settings: Mutex::new(settings),
            revision: Default::default(),
            env: RwLock::new(env),
            ops: Default::default(),
            locks: Default::default(),
            tokens,
            quick: Default::default(),
            forge: Default::default(),
            accounts_lock: Default::default(),
        }
    }

    /// The app's state from its config dir, the login shell's PATH and the system's token store.
    pub fn boot() -> Result<AppState> {
        let config_dir = paths::config_dir()?;
        let settings = Settings::load(&config_dir);
        let (env, _) = Env::detect(settings.git_path.as_deref());
        let tokens = Arc::new(SystemStore::default());
        Ok(AppState::new(config_dir, settings, env, tokens))
    }

    pub fn env(&self) -> Env {
        self.env.read().unwrap().clone()
    }

    /// Runs `f` as the operation `id`, which `ops.cancel(id)` cancels meanwhile.
    pub fn op<T>(&self, id: u64, f: impl FnOnce(&Cancel) -> T) -> T {
        let cancel = self.ops.start(id);
        let r = f(&cancel);
        self.ops.finish(id);
        r
    }
}

pub type Cancel = Arc<AtomicBool>;

/// Cancellable operations by op id.
#[derive(Default)]
pub struct Ops {
    last: AtomicU64,
    flags: Mutex<HashMap<u64, Cancel>>,
}

impl Ops {
    /// A new id, registered now so a cancel before the op starts still counts.
    pub fn id(&self) -> u64 {
        let id = self.last.fetch_add(1, Ordering::Relaxed) + 1;
        self.flags.lock().unwrap().insert(id, Cancel::default());
        id
    }

    pub fn start(&self, id: u64) -> Cancel {
        self.flags.lock().unwrap().entry(id).or_default().clone()
    }

    pub fn cancel(&self, id: u64) {
        if let Some(c) = self.flags.lock().unwrap().get(&id) {
            c.store(true, Ordering::SeqCst);
        }
    }

    pub fn finish(&self, id: u64) {
        self.flags.lock().unwrap().remove(&id);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cancel_before_the_op_starts_is_kept() {
        let ops = Ops::default();
        let (a, b) = (ops.id(), ops.id());
        assert_ne!(a, b);
        ops.cancel(a);
        assert!(ops.start(a).load(Ordering::SeqCst));
        assert!(!ops.start(b).load(Ordering::SeqCst));
        ops.finish(a);
        assert!(
            !ops.start(a).load(Ordering::SeqCst),
            "a finished id starts afresh"
        );
    }
}
