//! The agents' adapters: a catalog check at launch and daily, which updates an installed adapter
//! once a new version is out of its cool-down and never installs a first one, and the Agents page's
//! install, rollback and CLI check.

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::time::Duration;

use detached_core::adapters::{self, Catalog, Cli, Config, Entry, Installed, PROVIDERS, Provider};
use detached_core::adapters::{BLOCK, Version};
use detached_core::error::{Error, Result};
use detached_core::state::{AppState, Cancel};
use detached_core::update::Progress;
use gpui::{App, AppContext, Context, Entity, Global};

use crate::blocking;

const DAY: Duration = Duration::from_secs(24 * 60 * 60);

pub struct Agents {
    pub url: String,
    pub key: String,
    pub app: Version,
    /// The last catalog accepted, fetched or stored.
    pub catalog: Option<Catalog>,
    /// The last check's error; the catalog stays.
    pub error: Option<String>,
    pub checking: bool,
    pub config: Config,
    /// One per provider, in `PROVIDERS`' order.
    pub rows: Vec<Row>,
}

pub struct Row {
    pub provider: &'static Provider,
    pub installed: Installed,
    /// The CLI found and the version it printed.
    pub cli: Option<(PathBuf, Option<Version>)>,
    pub state: State,
    cancel: Option<Cancel>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum State {
    Idle,
    Downloading { received: u64, total: Option<u64> },
    Verifying,
    Failed(String),
}

impl Row {
    pub fn busy(&self) -> bool {
        matches!(self.state, State::Downloading { .. } | State::Verifying)
    }

    /// The catalog's entry of the version in use, whose `min_cli` and `blocked_cli` hold.
    pub fn entry<'a>(&self, catalog: Option<&'a Catalog>) -> Option<&'a Entry> {
        catalog?.entry(self.provider.adapter, self.installed.current.as_ref()?)
    }

    /// What the CLI's version allows, once one is found.
    pub fn verdict(&self, catalog: Option<&Catalog>) -> Option<Cli> {
        let version = self.cli.as_ref()?.1.as_ref()?;
        Some(adapters::check_cli(
            self.provider,
            self.entry(catalog),
            version,
        ))
    }
}

struct GlobalAgents(Entity<Agents>);

impl Global for GlobalAgents {}

pub(crate) fn init(cx: &mut App) {
    let rows = PROVIDERS.iter().map(|&provider| Row {
        provider,
        installed: Installed::default(),
        cli: None,
        state: State::Idle,
        cancel: None,
    });
    let agents = cx.new(|_| Agents {
        url: adapters::CATALOG_URL.into(),
        key: adapters::CATALOG_KEY.into(),
        app: adapters::app_version(),
        catalog: None,
        error: None,
        checking: false,
        config: Config::default(),
        rows: rows.collect(),
    });
    cx.set_global(GlobalAgents(agents));
}

/// Checks now and then daily.
pub fn start(cx: &mut App) {
    let agents = Agents::global(cx);
    cx.spawn(async move |cx| {
        loop {
            agents.update(cx, |a, cx| a.check(cx));
            cx.background_executor().timer(DAY).await;
        }
    })
    .detach();
}

type Scan = (Config, Vec<(Installed, Option<(PathBuf, Option<Version>)>)>);

/// What is installed and which CLI each listed provider has.
fn scan(core: &AppState) -> Scan {
    let config: Config = core.get_settings().block(BLOCK).unwrap_or_default();
    let env = core.env();
    let rows = PROVIDERS.iter().map(|p| {
        let setting = config.cli.get(p.id).map(String::as_str);
        let cli = config
            .shows(p)
            .then(|| adapters::find_cli(p, setting, &env));
        let cli = cli.flatten().map(|c| {
            let version = adapters::cli_version(&c, &env).ok();
            (c, version)
        });
        (adapters::installed(&core.data_dir, p.adapter), cli)
    });
    let rows = rows.collect();
    (config, rows)
}

impl Agents {
    pub fn global(cx: &App) -> Entity<Agents> {
        cx.global::<GlobalAgents>().0.clone()
    }

    pub fn row(&self, id: &str) -> Option<&Row> {
        self.rows.iter().find(|r| r.provider.id == id)
    }

    fn row_mut(&mut self, id: &str) -> Option<&mut Row> {
        self.rows.iter_mut().find(|r| r.provider.id == id)
    }

    /// The newest entry the provider's adapter can install by hand, cool-down or not.
    pub fn offer(&self, id: &str) -> Option<&Entry> {
        let row = self.row(id)?;
        let newest = self
            .catalog
            .as_ref()?
            .entries(row.provider.adapter, &self.app);
        newest.into_iter().next()
    }

    fn apply(&mut self, (config, rows): Scan) {
        self.config = config;
        for (row, (installed, cli)) in self.rows.iter_mut().zip(rows) {
            (row.installed, row.cli) = (installed, cli);
        }
    }

    /// Reads what is installed and finds the CLIs, without the network.
    pub fn load(&mut self, cx: &mut Context<Self>) {
        let core = crate::core(cx).clone();
        let done = blocking::run(cx, move || {
            let stored = adapters::stored(&core.data_dir);
            (scan(&core), stored)
        });
        cx.spawn(async move |this, cx| {
            let (scan, stored) = done.await;
            this.update(cx, |a, cx| {
                a.apply(scan);
                if a.catalog.is_none() {
                    a.catalog = stored;
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Fetches the catalog, then updates each installed adapter that has a version out of its
    /// cool-down.
    pub fn check(&mut self, cx: &mut Context<Self>) {
        if self.checking {
            return;
        }
        self.checking = true;
        cx.notify();
        let core = crate::core(cx).clone();
        let (url, key) = (self.url.clone(), self.key.clone());
        let done = blocking::run(cx, move || {
            let fetched = adapters::fetch(&url, &key, &core.data_dir);
            let catalog = fetched.as_ref().ok().cloned();
            let stored = catalog.or_else(|| adapters::stored(&core.data_dir));
            (fetched.err(), stored, scan(&core))
        });
        cx.spawn(async move |this, cx| {
            let (error, catalog, scan) = done.await;
            this.update(cx, |a, cx| {
                a.checking = false;
                a.error = error.map(|e| e.to_string());
                a.catalog = catalog.or(a.catalog.take());
                a.apply(scan);
                let today = adapters::today();
                let due: Vec<_> = (a.rows.iter())
                    .filter(|r| !r.busy())
                    .filter_map(|r| {
                        let catalog = a.catalog.as_ref()?;
                        let e = catalog.auto(r.provider.adapter, &a.app, &r.installed, today)?;
                        Some((r.provider.id, e.clone()))
                    })
                    .collect();
                for (id, entry) in due {
                    a.install_entry(id, entry, cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Installs the newest version at once.
    pub fn install(&mut self, id: &'static str, cx: &mut Context<Self>) {
        if let Some(entry) = self.offer(id).cloned() {
            self.install_entry(id, entry, cx);
        }
    }

    fn install_entry(&mut self, id: &'static str, entry: Entry, cx: &mut Context<Self>) {
        let Some(row) = self.row_mut(id).filter(|r| !r.busy()) else {
            return;
        };
        let cancel = Cancel::default();
        row.cancel = Some(cancel.clone());
        row.state = State::Downloading {
            received: 0,
            total: None,
        };
        cx.notify();
        let data = crate::core(cx).data_dir.clone();
        let weak = cx.weak_entity();
        let done = blocking::stream(
            cx,
            move |send| adapters::install(&data, &entry, &cancel, send),
            move |batch: Vec<Progress>, cx| {
                let state = match batch.last() {
                    Some(Progress::Received(received, total)) => State::Downloading {
                        received: *received,
                        total: *total,
                    },
                    _ => State::Verifying,
                };
                weak.update(cx, |a, cx| {
                    if let Some(row) = a.row_mut(id) {
                        row.state = state;
                    }
                    cx.notify();
                })
                .ok();
            },
        );
        self.finish(id, done, cx);
    }

    fn finish(
        &mut self,
        id: &'static str,
        done: gpui::Task<Result<Installed>>,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |this, cx| {
            let done = done.await;
            this.update(cx, |a, cx| {
                if let Some(row) = a.row_mut(id) {
                    row.cancel = None;
                    row.state = match done {
                        Ok(kept) => {
                            row.installed = kept;
                            State::Idle
                        }
                        Err(Error::Cancelled) => State::Idle,
                        Err(e) => State::Failed(e.to_string()),
                    };
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Makes the kept version current again.
    pub fn rollback(&mut self, id: &'static str, cx: &mut Context<Self>) {
        let Some(row) = self.row(id).filter(|r| !r.busy()) else {
            return;
        };
        let (data, name) = (crate::core(cx).data_dir.clone(), row.provider.adapter);
        let done = blocking::run(cx, move || adapters::rollback(&data, name));
        self.finish(id, done, cx);
    }

    pub fn cancel(&self, id: &str) {
        if let Some(c) = self.row(id).and_then(|r| r.cancel.as_ref()) {
            c.store(true, Ordering::Relaxed);
        }
    }

    /// Uses the CLI at `path` for the provider, or the one on PATH when it is empty.
    pub fn set_cli(&mut self, id: &'static str, path: String, cx: &mut Context<Self>) {
        let core = crate::core(cx).clone();
        let done = blocking::run(cx, move || {
            let mut config: Config = core.get_settings().block(BLOCK).unwrap_or_default();
            match path.trim() {
                "" => config.cli.remove(id),
                path => config.cli.insert(id.into(), path.into()),
            };
            core.set_block(BLOCK, config)?;
            Ok(scan(&core))
        });
        cx.spawn(async move |this, cx| {
            let done: Result<Scan> = done.await;
            this.update(cx, |a, cx| {
                match done {
                    Ok(scan) => a.apply(scan),
                    Err(e) => a.error = Some(e.to_string()),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use detached_core::adapters::testutil::Fixture;
    use detached_core::adapters::{CLAUDE, Config};
    use gpui::TestAppContext;

    use super::*;
    use crate::wait_until;

    fn version(cx: &mut TestAppContext) -> Option<String> {
        cx.read(|cx| {
            let a = Agents::global(cx).read(cx);
            let current = a.row(CLAUDE.id)?.installed.current.as_ref();
            current.map(|v| v.to_string())
        })
    }

    fn check(cx: &mut TestAppContext) {
        let agents = cx.read(Agents::global);
        agents.update(cx, |a, cx| a.check(cx));
        wait_until(cx, |cx| {
            let a = Agents::global(cx).read(cx);
            !a.checking && a.rows.iter().all(|r| !r.busy())
        });
    }

    #[gpui::test]
    fn a_check_installs_no_first_adapter_and_updates_one_past_its_cool_down(
        cx: &mut TestAppContext,
    ) {
        let _dir = crate::boot(cx);
        let f = Fixture::new();
        let missing = Config {
            cli: [(CLAUDE.id.into(), "/nowhere/claude".into())].into(),
            codex: false,
        };
        cx.read(|cx| crate::core(cx).set_block(BLOCK, missing))
            .unwrap();
        let agents = cx.read(Agents::global);
        agents.update(cx, |a, _| (a.url, a.key) = (f.url(), f.key.clone()));
        f.publish(1, &[f.entry(CLAUDE.adapter, "0.1.0")]);
        check(cx);
        assert_eq!(version(cx), None);
        cx.read(|cx| assert!(Agents::global(cx).read(cx).error.is_none()));
        agents.update(cx, |a, cx| a.install(CLAUDE.id, cx));
        wait_until(cx, |cx| {
            let a = Agents::global(cx).read(cx);
            a.row(CLAUDE.id).unwrap().installed.current.is_some()
        });
        let mut fresh = f.entry(CLAUDE.adapter, "0.3.0");
        fresh.published = adapters::today();
        f.publish(2, &[f.entry(CLAUDE.adapter, "0.2.0"), fresh]);
        check(cx);
        assert_eq!(version(cx).as_deref(), Some("0.2.0"));
        cx.read(|cx| {
            let a = Agents::global(cx).read(cx);
            assert_eq!(a.offer(CLAUDE.id).unwrap().version.to_string(), "0.3.0");
            assert_eq!(a.row(CLAUDE.id).unwrap().cli, None);
        });
    }
}
