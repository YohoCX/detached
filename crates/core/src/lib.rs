pub mod accounts;
pub mod adapters;
pub mod api;
// A backend's protocol stays in a private module of its crate; only its constructor leaves it.
pub mod backend;
pub mod chat_index;
pub mod chat_settings;
mod claude;
pub mod council;
pub mod crash;
pub mod docs;
pub mod env;
pub mod error;
pub mod forge;
pub mod git;
pub mod host;
pub mod os;
pub mod owner;
pub mod paths;
pub mod plugins;
pub mod proc;
pub mod processes;
pub mod search;
pub mod secrets;
#[cfg(unix)]
pub mod selftest;
pub mod sessions;
pub mod settings;
pub mod skills;
pub mod state;
pub mod store;
#[cfg(any(test, feature = "testutil"))]
pub mod testutil;
pub mod trust;
pub mod types;
pub mod update;
pub mod watch;
