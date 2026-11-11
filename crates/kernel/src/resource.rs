//! Values loaded through core and kept for the session, each reloaded by its policy while an
//! interest holds it.

use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

use detached_core::error::Result;
use gpui::{AnyEntity, App, AppContext, Context, Entity, Global, Subscription, Task};

use crate::blocking;

pub struct Policy<T> {
    /// Window activation reloads a value older than this; None leaves it to its other inputs.
    pub fresh_for: Option<Duration>,
    /// Reloads every period while the value meets the condition, an interest is held and the
    /// window shows.
    pub poll: Option<(Duration, When<T>)>,
}

pub type When<T> = fn(&T) -> bool;

/// What else marks a resource stale.
#[derive(Clone, PartialEq)]
pub enum On {
    /// The repo's status loaded: its worktree may have changed.
    Status(String),
    /// The repo's head or branch moved.
    Head(String),
}

type Job<T> = Box<dyn FnOnce() -> std::result::Result<T, String> + Send>;
type Load<T> = Box<dyn Fn(&App) -> Job<T>>;

pub struct Resource<T> {
    pub value: Option<T>,
    /// The last load's error; the value stays.
    pub error: Option<String>,
    /// When the last load ended.
    pub at: Option<Instant>,
    policy: Policy<T>,
    on: Option<On>,
    load: Load<T>,
    loading: Option<Task<()>>,
    held: Rc<()>,
    polling: bool,
}

/// Keeps a resource loaded while it lives.
pub struct Interest<T> {
    resource: Entity<Resource<T>>,
    _held: Rc<()>,
    _redraw: Option<Subscription>,
}

/// What reaches every resource.
pub(crate) enum Input {
    Activated,
    On(On),
}

type Apply = fn(AnyEntity, &Input, &mut App);

#[derive(Default)]
struct Resources(HashMap<String, (AnyEntity, Apply)>);

impl Global for Resources {}

/// The resource under `key`, made by `make` the first time.
pub fn get<T: Send + 'static>(
    cx: &mut App,
    key: String,
    make: impl FnOnce() -> Resource<T>,
) -> Entity<Resource<T>> {
    if let Some((e, _)) = cx.default_global::<Resources>().0.get(&key) {
        return e.clone().downcast().expect("one type per key");
    }
    let e = cx.new(|_| make());
    let apply: Apply = |e, input, cx| {
        if let Ok(e) = e.downcast::<Resource<T>>() {
            e.update(cx, |r, cx| r.input(input, cx));
        }
    };
    let entry = (e.clone().into_any(), apply);
    cx.default_global::<Resources>().0.insert(key, entry);
    e
}

pub(crate) fn input(cx: &mut App, input: Input) {
    let all: Vec<_> = cx
        .try_global::<Resources>()
        .map_or(vec![], |r| r.0.values().cloned().collect());
    for (e, apply) in all {
        apply(e, &input, cx);
    }
}

impl<T: Send + 'static> Resource<T> {
    pub fn new<F>(policy: Policy<T>, load: impl Fn(&App) -> F + 'static) -> Self
    where
        F: FnOnce() -> Result<T> + Send + 'static,
    {
        Resource {
            value: None,
            error: None,
            at: None,
            policy,
            on: None,
            load: Box::new(move |cx| {
                let f = load(cx);
                Box::new(move || f().map_err(|e| e.to_string()))
            }),
            loading: None,
            held: Rc::default(),
            polling: false,
        }
    }

    pub fn on(self, on: On) -> Self {
        Resource {
            on: Some(on),
            ..self
        }
    }

    pub fn loading(&self) -> bool {
        self.loading.is_some()
    }

    fn held(&self) -> bool {
        Rc::strong_count(&self.held) > 1
    }

    /// Loads now, whatever its age; a load still running is dropped.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let done = blocking::run(cx, (self.load)(cx));
        self.loading = Some(cx.spawn(async move |this, cx| {
            let r = done.await;
            this.update(cx, |this, cx| {
                this.loading = None;
                this.at = Some(cx.background_executor().now());
                match r {
                    Ok(v) => (this.value, this.error) = (Some(v), None),
                    Err(e) => this.error = Some(e),
                }
                this.poll(cx);
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn input(&mut self, input: &Input, cx: &mut Context<Self>) {
        let stale = match input {
            Input::Activated => {
                let now = cx.background_executor().now();
                let old = |f| self.at.is_none_or(|at| now.duration_since(at) >= f);
                self.loading.is_none() && self.policy.fresh_for.is_some_and(old)
            }
            Input::On(on) => self.on.as_ref() == Some(on),
        };
        if stale && self.held() {
            self.refresh(cx);
        }
    }

    /// Starts the policy's poll unless it runs; it stops at a tick that finds its conditions gone.
    fn poll(&mut self, cx: &mut Context<Self>) {
        let Some((every, when)) = self.policy.poll else {
            return;
        };
        let on = move |r: &Self| r.held() && r.value.as_ref().is_some_and(when);
        if self.polling || !on(self) {
            return;
        }
        self.polling = true;
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(every).await;
                let go = this.update(cx, |r, cx| {
                    r.polling = on(r);
                    if r.polling && r.loading.is_none() && crate::invalidate::visible(cx) {
                        r.refresh(cx);
                    }
                    r.polling
                });
                if !go.unwrap_or(false) {
                    return;
                }
            }
        })
        .detach();
    }
}

impl<T: Send + 'static> Interest<T> {
    /// Loads `resource` now; activation and its poll reload it while this lives.
    pub fn new(resource: Entity<Resource<T>>, cx: &mut App) -> Self {
        let held = resource.update(cx, |r, cx| {
            let held = r.held.clone();
            r.refresh(cx);
            r.poll(cx);
            held
        });
        Interest {
            resource,
            _held: held,
            _redraw: None,
        }
    }

    /// An interest of the view `cx`, which redraws when the resource changes.
    pub fn view<V: 'static>(resource: Entity<Resource<T>>, cx: &mut Context<V>) -> Self {
        let redraw = cx.observe(&resource, |_, _, cx| cx.notify());
        Interest {
            _redraw: Some(redraw),
            ..Interest::new(resource, cx)
        }
    }

    pub fn resource(&self) -> &Entity<Resource<T>> {
        &self.resource
    }

    pub fn read<'a>(&self, cx: &'a App) -> &'a Resource<T> {
        self.resource.read(cx)
    }

    /// For a Refresh button: loads now, whatever its age.
    pub fn refresh(&self, cx: &mut App) {
        self.resource.update(cx, |r, cx| r.refresh(cx));
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::SeqCst};

    use detached_core::error::Error;
    use gpui::TestAppContext;

    use super::*;
    use crate::{invalidate, wait_until};

    /// A resource whose value counts its loads; a load fails while `fail` is set.
    fn counter(
        cx: &mut TestAppContext,
        policy: Policy<usize>,
    ) -> (Entity<Resource<usize>>, Arc<AtomicUsize>, Arc<AtomicBool>) {
        let (n, fail) = (
            Arc::new(AtomicUsize::new(0)),
            Arc::new(AtomicBool::new(false)),
        );
        let (loads, failing) = (n.clone(), fail.clone());
        let r = cx.update(|cx| {
            get(cx, "counter".into(), || {
                Resource::new(policy, move |_| {
                    let (n, fail) = (loads.clone(), failing.clone());
                    move || match fail.load(SeqCst) {
                        true => Err(Error::Other("down".into())),
                        false => Ok(n.fetch_add(1, SeqCst) + 1),
                    }
                })
            })
        });
        (r, n, fail)
    }

    fn interest(cx: &mut TestAppContext, r: &Entity<Resource<usize>>) -> Interest<usize> {
        let i = cx.update(|cx| Interest::new(r.clone(), cx));
        settle(cx, r);
        i
    }

    fn settle(cx: &mut TestAppContext, r: &Entity<Resource<usize>>) {
        wait_until(cx, |cx| !r.read(cx).loading());
    }

    fn value(cx: &mut TestAppContext, r: &Entity<Resource<usize>>) -> Option<usize> {
        r.read_with(cx, |r, _| r.value)
    }

    /// Moves the clock on and says whether that loaded `r`.
    fn advance(cx: &mut TestAppContext, r: &Entity<Resource<usize>>, secs: u64) -> bool {
        let before = value(cx, r);
        cx.executor().advance_clock(Duration::from_secs(secs));
        settle(cx, r);
        value(cx, r) != before
    }

    #[gpui::test]
    fn activation_skips_a_resource_fresher_than_its_policy_and_refresh_never_skips(
        cx: &mut TestAppContext,
    ) {
        let _dir = crate::boot(cx);
        let fresh = Duration::from_secs(30);
        let policy = Policy {
            fresh_for: Some(fresh),
            poll: None,
        };
        let (r, ..) = counter(cx, policy);
        let held = interest(cx, &r);
        let activate = |cx: &mut TestAppContext| {
            let before = value(cx, &r);
            cx.update(invalidate::activated);
            settle(cx, &r);
            value(cx, &r) != before
        };
        cx.executor().advance_clock(fresh - Duration::from_secs(1));
        assert!(!activate(cx));
        cx.update(|cx| held.refresh(cx));
        settle(cx, &r);
        assert_eq!(value(cx, &r), Some(2), "a refresh does not skip");
        cx.executor().advance_clock(fresh);
        assert!(activate(cx));
        assert_eq!(value(cx, &r), Some(3));
        drop(held);
        cx.executor().advance_clock(fresh);
        assert!(!activate(cx), "nothing holds it");
    }

    #[gpui::test]
    fn a_poll_ticks_while_an_interest_is_held_and_the_window_shows(cx: &mut TestAppContext) {
        let _dir = crate::boot(cx);
        let policy = Policy {
            fresh_for: None,
            poll: Some((Duration::from_secs(5), |_| true)),
        };
        let (r, n, _) = counter(cx, policy);
        let held = interest(cx, &r);
        assert!(advance(cx, &r, 5));
        cx.update(|cx| invalidate::set_visible(cx, false));
        assert!(!advance(cx, &r, 5), "the window is hidden");
        cx.update(|cx| invalidate::set_visible(cx, true));
        assert!(advance(cx, &r, 5));
        drop(held);
        assert!(!advance(cx, &r, 5), "a dropped interest stops its poll");
        assert!(!advance(cx, &r, 5));
        assert_eq!(n.load(SeqCst), 3);
        let _held = interest(cx, &r);
        assert!(advance(cx, &r, 5), "a new interest polls again");
    }

    #[gpui::test]
    fn a_failed_refresh_keeps_the_last_value_and_the_error_beside_it(cx: &mut TestAppContext) {
        let _dir = crate::boot(cx);
        let policy = Policy {
            fresh_for: None,
            poll: None,
        };
        let (r, _, fail) = counter(cx, policy);
        let held = interest(cx, &r);
        fail.store(true, SeqCst);
        cx.update(|cx| held.refresh(cx));
        settle(cx, &r);
        let got = r.read_with(cx, |r, _| (r.value, r.error.clone()));
        assert_eq!(got, (Some(1), Some("down".into())));
        fail.store(false, SeqCst);
        cx.update(|cx| held.refresh(cx));
        settle(cx, &r);
        let got = r.read_with(cx, |r, _| (r.value, r.error.clone()));
        assert_eq!(got, (Some(2), None));
    }
}
