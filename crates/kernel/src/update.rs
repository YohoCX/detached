//! The updater: checks at launch and daily, the download and the restart into a new release. It
//! alone holds the download's progress and its cancel; neither is a job in `Activity`.

use std::env;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::time::Duration;

use detached_core::error::Error;
use detached_core::state::Cancel;
use detached_core::update::{self, Install, Progress, Release};
use gpui::{App, AppContext, Context, Entity, Global};
use tempfile::TempPath;

use crate::blocking;

const DAY: Duration = Duration::from_secs(24 * 60 * 60);

/// Where releases come from and where a package downloads to.
pub struct Feed {
    pub base: String,
    pub key: String,
    /// The running version.
    pub version: String,
    pub dir: PathBuf,
}

impl Default for Feed {
    fn default() -> Feed {
        Feed {
            base: update::LATEST.into(),
            key: update::PUBLIC_KEY.into(),
            version: env!("CARGO_PKG_VERSION").into(),
            dir: env::temp_dir(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum State {
    Idle,
    Checking,
    UpToDate,
    Offered,
    Downloading {
        received: u64,
        total: Option<u64>,
    },
    Verifying,
    /// Downloaded and verified; installed by `restart`.
    Ready,
    Installing,
    Failed(String),
}

impl State {
    pub fn percent(&self) -> Option<u64> {
        match self {
            State::Downloading {
                received,
                total: Some(t),
            } if *t > 0 => Some((received * 100 / t).min(100)),
            _ => None,
        }
    }
}

pub struct Update {
    /// None when this build is not an install, and updates are off.
    pub install: Option<Install>,
    pub feed: Feed,
    pub state: State,
    /// The newest release a check found; a failed check keeps it.
    pub release: Option<Release>,
    cancel: Option<Cancel>,
    package: Option<TempPath>,
}

struct GlobalUpdate(Entity<Update>);

impl Global for GlobalUpdate {}

pub(crate) fn init(cx: &mut App) {
    let update = cx.new(|_| Update {
        install: Install::current(),
        feed: Feed::default(),
        state: State::Idle,
        release: None,
        cancel: None,
        package: None,
    });
    cx.set_global(GlobalUpdate(update));
}

/// Checks now and then daily, an offer or not.
pub fn start(cx: &mut App) {
    let update = Update::global(cx);
    cx.spawn(async move |cx| {
        loop {
            update.update(cx, |u, cx| u.check(cx));
            cx.background_executor().timer(DAY).await;
        }
    })
    .detach();
}

impl Update {
    pub fn global(cx: &App) -> Entity<Update> {
        cx.global::<GlobalUpdate>().0.clone()
    }

    /// Whether a check may start: not while one runs, nor from downloading to the restart.
    pub fn can_check(&self) -> bool {
        let idle = matches!(
            self.state,
            State::Idle | State::UpToDate | State::Offered | State::Failed(_)
        );
        self.install.is_some() && idle
    }

    pub fn check(&mut self, cx: &mut Context<Self>) {
        if !self.can_check() {
            return;
        }
        self.state = State::Checking;
        cx.notify();
        let f = &self.feed;
        let (base, version, key) = (f.base.clone(), f.version.clone(), f.key.clone());
        let found = blocking::run(cx, move || update::check(&base, &version, &key));
        cx.spawn(async move |this, cx| {
            let found = found.await;
            this.update(cx, |u, cx| {
                match found {
                    Ok(Some(r)) => (u.state, u.release) = (State::Offered, Some(r)),
                    Ok(None) => (u.state, u.release) = (State::UpToDate, None),
                    Err(e) => u.state = State::Failed(e.to_string()),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Downloads and verifies the offered release; a `.deb` never downloads.
    pub fn download(&mut self, cx: &mut Context<Self>) {
        let offered = matches!(self.state, State::Offered | State::Failed(_));
        let Some(release) = self.release.clone().filter(|_| offered) else {
            return;
        };
        if matches!(self.install, None | Some(Install::Deb)) {
            return;
        }
        let cancel = Cancel::default();
        self.cancel = Some(cancel.clone());
        self.state = State::Downloading {
            received: 0,
            total: None,
        };
        cx.notify();
        let (dir, key) = (self.feed.dir.clone(), self.feed.key.clone());
        let weak = cx.weak_entity();
        let done = blocking::stream(
            cx,
            move |send| update::download(&release, &key, &dir, &cancel, send),
            move |batch: Vec<Progress>, cx| {
                let state = match batch.last() {
                    Some(Progress::Received(received, total)) => State::Downloading {
                        received: *received,
                        total: *total,
                    },
                    _ => State::Verifying,
                };
                weak.update(cx, |u, cx| {
                    u.state = state;
                    cx.notify();
                })
                .ok();
            },
        );
        cx.spawn(async move |this, cx| {
            let done = done.await;
            this.update(cx, |u, cx| {
                u.cancel = None;
                u.state = match done {
                    Ok(package) => {
                        u.package = Some(package);
                        State::Ready
                    }
                    Err(Error::Cancelled) => State::Offered,
                    Err(e) => State::Failed(e.to_string()),
                };
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Stops the download; it is offered again once the download has ended.
    pub fn cancel(&self) {
        if let Some(c) = &self.cancel {
            c.store(true, Ordering::Relaxed);
        }
    }

    /// Installs the ready package and quits, for the new version to start.
    pub fn restart(&mut self, cx: &mut Context<Self>) {
        if self.state != State::Ready {
            return;
        }
        let (Some(install), Some(package)) = (self.install.clone(), self.package.take()) else {
            return;
        };
        self.state = State::Installing;
        cx.notify();
        let applied = blocking::run(cx, move || install.apply(&package));
        cx.spawn(async move |this, cx| match applied.await {
            Ok(()) => cx.update(|cx| cx.quit()),
            Err(e) => {
                this.update(cx, |u, cx| {
                    u.state = State::Failed(e.to_string());
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use detached_core::update::testutil::{Server, keys};
    use gpui::TestAppContext;
    use tempfile::TempDir;

    use super::*;
    use crate::wait_until;

    /// The kernel with an update feed on `server`, as an install of `install` at version 0.3.0;
    /// the second dir is where packages download.
    fn boot(cx: &mut TestAppContext, server: &Server, key: &str, install: Install) -> [TempDir; 2] {
        let dir = crate::boot(cx);
        let packages = tempfile::tempdir().unwrap();
        let feed = Feed {
            base: server.base.clone(),
            key: key.into(),
            version: "0.3.0".into(),
            dir: packages.path().into(),
        };
        update(cx, |u, _| (u.install, u.feed) = (Some(install), feed));
        [dir, packages]
    }

    fn update<R>(
        cx: &mut TestAppContext,
        f: impl FnOnce(&mut Update, &mut Context<Update>) -> R,
    ) -> R {
        cx.update(|cx| Update::global(cx).update(cx, f))
    }

    fn state(cx: &TestAppContext) -> State {
        cx.read(|cx| Update::global(cx).read(cx).state.clone())
    }

    fn wait_for(cx: &mut TestAppContext, done: impl Fn(&State) -> bool) {
        wait_until(cx, |cx| done(&Update::global(cx).read(cx).state));
    }

    fn version(cx: &TestAppContext) -> Option<String> {
        cx.read(|cx| {
            let r = Update::global(cx).read(cx).release.clone();
            r.map(|r| r.version.to_string())
        })
    }

    fn files(dir: &TempDir) -> usize {
        fs::read_dir(dir.path()).unwrap().count()
    }

    #[gpui::test]
    fn a_cancel_mid_download_offers_again_leaves_no_file_and_a_new_download_starts(
        cx: &mut TestAppContext,
    ) {
        let (key, sign) = keys();
        let package = vec![7; 256 << 10];
        let server = Server::start(&sign, "9.9.9", &package, &sign(&package));
        let [_dir, packages] = boot(cx, &server, &key, Install::Windows);
        update(cx, |u, cx| u.check(cx));
        wait_for(cx, |s| *s == State::Offered);
        let release = server.hold();
        update(cx, |u, cx| u.download(cx));
        let half = package.len() as u64 / 2;
        wait_for(
            cx,
            |s| matches!(s, State::Downloading { received, .. } if *received == half),
        );
        assert_eq!(state(cx).percent(), Some(50));
        update(cx, |u, cx| {
            u.cancel();
            u.download(cx);
        });
        assert!(
            matches!(state(cx), State::Downloading { .. }),
            "nothing new until it ends"
        );
        drop(release);
        wait_for(cx, |s| *s == State::Offered);
        assert_eq!(files(&packages), 0);
        update(cx, |u, cx| u.download(cx));
        wait_for(cx, |s| *s == State::Ready);
        assert_eq!(files(&packages), 1);
    }

    #[gpui::test]
    fn a_click_while_a_download_runs_starts_nothing(cx: &mut TestAppContext) {
        let (key, sign) = keys();
        let package = vec![7; 256 << 10];
        let server = Server::start(&sign, "9.9.9", &package, &sign(&package));
        let [_dir, packages] = boot(cx, &server, &key, Install::Windows);
        update(cx, |u, cx| u.check(cx));
        wait_for(cx, |s| *s == State::Offered);
        let release = server.hold();
        update(cx, |u, cx| u.download(cx));
        wait_for(
            cx,
            |s| matches!(s, State::Downloading { received, .. } if *received > 0),
        );
        update(cx, |u, cx| {
            u.download(cx);
            u.check(cx);
        });
        assert!(matches!(state(cx), State::Downloading { .. }));
        drop(release);
        wait_for(cx, |s| *s == State::Ready);
        assert_eq!(files(&packages), 1);
    }

    #[gpui::test]
    fn a_bad_signature_fails_with_its_text_and_no_file(cx: &mut TestAppContext) {
        let (key, sign) = keys();
        let server = Server::start(&sign, "9.9.9", b"tampered", &sign(b"signed"));
        let [_dir, packages] = boot(cx, &server, &key, Install::Windows);
        update(cx, |u, cx| u.check(cx));
        wait_for(cx, |s| *s == State::Offered);
        update(cx, |u, cx| u.download(cx));
        wait_for(cx, |s| matches!(s, State::Failed(_)));
        let State::Failed(text) = state(cx) else {
            unreachable!()
        };
        assert!(text.starts_with("signature: "), "{text}");
        assert_eq!(files(&packages), 0);
        assert_eq!(version(cx).as_deref(), Some("9.9.9"), "still offered");
    }

    #[gpui::test]
    fn checks_go_on_daily_after_an_offer_and_find_a_newer_release(cx: &mut TestAppContext) {
        let (key, sign) = keys();
        let server = Server::start(&sign, "9.9.9", b"p", &sign(b"p"));
        let _dirs = boot(cx, &server, &key, Install::Windows);
        cx.update(start);
        wait_for(cx, |s| *s == State::Offered);
        assert_eq!(version(cx).as_deref(), Some("9.9.9"));
        server.offer(&sign, "10.0.0", b"p", &sign(b"p"));
        cx.executor().advance_clock(DAY);
        wait_until(cx, |cx| {
            let r = Update::global(cx).read(cx).release.clone();
            r.is_some_and(|r| r.version.major == 10)
        });
        assert_eq!(state(cx), State::Offered);
    }

    #[gpui::test]
    fn a_check_reports_up_to_date_and_an_error(cx: &mut TestAppContext) {
        let (key, sign) = keys();
        let server = Server::start(&sign, "0.3.0", b"", "");
        let _dirs = boot(cx, &server, &key, Install::Windows);
        update(cx, |u, cx| u.check(cx));
        assert_eq!(state(cx), State::Checking);
        wait_for(cx, |s| *s == State::UpToDate);
        update(cx, |u, cx| u.check(cx));
        wait_for(cx, |s| *s == State::UpToDate);
        server.offer(keys().1, "9.9.9", b"", "");
        update(cx, |u, cx| u.check(cx));
        wait_for(
            cx,
            |s| matches!(s, State::Failed(t) if t.starts_with("signature: ")),
        );
        assert_eq!(version(cx), None);
    }

    #[gpui::test]
    fn a_deb_never_downloads(cx: &mut TestAppContext) {
        let (key, sign) = keys();
        let server = Server::start(&sign, "9.9.9", b"p", &sign(b"p"));
        let [_dir, packages] = boot(cx, &server, &key, Install::Deb);
        update(cx, |u, cx| u.check(cx));
        wait_for(cx, |s| *s == State::Offered);
        update(cx, |u, cx| u.download(cx));
        cx.run_until_parked();
        assert_eq!(state(cx), State::Offered);
        assert_eq!(files(&packages), 0);
    }

    #[gpui::test]
    fn a_build_that_is_not_an_install_never_checks(cx: &mut TestAppContext) {
        let _dir = crate::boot(cx);
        update(cx, |u, cx| {
            assert_eq!(u.install, None);
            u.check(cx);
        });
        assert_eq!(state(cx), State::Idle);
    }
}
