pub mod accounts;
pub mod adapters;
pub mod agent;
pub mod api;
// A backend's protocol stays in a private module of its crate; only its constructor leaves it.
pub mod backend;
mod claude;
pub mod crash;
pub mod env;
pub mod error;
pub mod forge;
pub mod git;
pub mod host;
pub mod os;
pub mod owner;
pub mod panel;
pub mod paths;
pub mod proc;
pub mod search;
#[cfg(unix)]
pub mod selftest;
pub mod settings;
pub mod state;
pub mod store;
#[cfg(any(test, feature = "testutil"))]
pub mod testutil;
pub mod trust;
pub mod types;
pub mod update;
pub mod watch;
