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

/// Sees None before a load succeeded.
pub type When<T> = fn(Option<&T>) -> bool;

/// What else marks a resource stale.
#[derive(Clone, PartialEq)]
pub enum On {
    ChatHistory,
    RecipeTrust,
    MrRemote(String),
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
    /// Loads ended so far; one runs at a time.
    loads: u64,
    /// The loads that must have ended for the last refresh to be served: a load already running
    /// when it came read before it.
    due: u64,
    policy: Policy<T>,
    on: Vec<On>,
    load: Load<T>,
    loading: Option<Task<()>>,
    /// A refresh came while a load ran: another follows it.
    again: bool,
    /// The value was edited while a load ran, which read before the edit: its value is dropped.
    edited: bool,
    discard_load: bool,
    held: Rc<()>,
    polling: bool,
}

/// Keeps a resource loaded while it lives.
pub struct Interest<T> {
    resource: Entity<Resource<T>>,
    /// The loads that must have ended for this interest's own load.
    due: u64,
    _held: Rc<()>,
    _redraw: Option<Subscription>,
}

/// What reaches every resource.
pub(crate) enum Input {
    Activated,
    On(On),
    Reset(On),
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
            loads: 0,
            due: 0,
            policy,
            on: Vec::new(),
            load: Box::new(move |cx| {
                let f = load(cx);
                Box::new(move || f().map_err(|e| e.to_string()))
            }),
            loading: None,
            again: false,
            edited: false,
            discard_load: false,
            held: Rc::default(),
            polling: false,
        }
    }

    pub fn on(mut self, on: On) -> Self {
        self.on.push(on);
        self
    }

    pub fn loading(&self) -> bool {
        self.loading.is_some()
    }

    fn held(&self) -> bool {
        Rc::strong_count(&self.held) > 1
    }

    /// Loads now, whatever its age; while a load runs, once it is done, every refresh meanwhile
    /// served by that one load. Dropping the running one instead would let refreshes that come
    /// faster than a load keep any from finishing.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.loading.is_some() {
            (self.again, self.due) = (true, self.loads + 2);
            return;
        }
        self.due = self.loads + 1;
        let done = blocking::run(cx, (self.load)(cx));
        self.loading = Some(cx.spawn(async move |this, cx| {
            let r = done.await;
            this.update(cx, |this, cx| {
                this.loading = None;
                this.loads += 1;
                if !std::mem::take(&mut this.discard_load) {
                    this.at = Some(cx.background_executor().now());
                    match r {
                        Ok(_) if this.edited => {}
                        Ok(v) => (this.value, this.error) = (Some(v), None),
                        Err(e) => this.error = Some(e),
                    }
                }
                this.edited = false;
                if std::mem::take(&mut this.again) {
                    this.refresh(cx);
                }
                this.poll(cx);
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    /// Changes the value at once, ahead of the write it stands for; the caller refreshes once that
    /// write is done.
    pub fn edit(&mut self, f: impl FnOnce(&mut T), cx: &mut Context<Self>) {
        if let Some(v) = &mut self.value {
            f(v);
        }
        self.edited = self.loading.is_some();
        cx.notify();
    }

    fn input(&mut self, input: &Input, cx: &mut Context<Self>) {
        let stale = match input {
            Input::Activated => {
                let now = cx.background_executor().now();
                let old = |f| self.at.is_none_or(|at| now.duration_since(at) >= f);
                self.loading.is_none() && self.policy.fresh_for.is_some_and(old)
            }
            Input::On(on) => self.on.contains(on),
            Input::Reset(on) => {
                if !self.on.contains(on) {
                    return;
                }
                (self.value, self.error, self.at) = (None, None, None);
                self.discard_load = self.loading();
                cx.notify();
                true
            }
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
        let on = move |r: &Self| r.held() && when(r.value.as_ref());
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
    /// Refreshes `resource`; activation and its poll reload it while this lives.
    pub fn new(resource: Entity<Resource<T>>, cx: &mut App) -> Self {
        let (held, due) = resource.update(cx, |r, cx| {
            r.refresh(cx);
            r.poll(cx);
            (r.held.clone(), r.due)
        });
        Interest {
            resource,
            due,
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

    /// The resource once a load that started after this interest was taken ended: a load already
    /// running then never serves it.
    pub fn loaded<'a>(&self, cx: &'a App) -> Option<&'a Resource<T>> {
        let r = self.read(cx);
        (r.loads >= self.due).then_some(r)
    }

    /// For a Refresh button: loads now, whatever its age, or once the running load is done.
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
    fn an_interest_taken_while_a_load_runs_is_served_by_the_load_after_it(cx: &mut TestAppContext) {
        let _dir = crate::boot(cx);
        let policy = Policy {
            fresh_for: None,
            poll: None,
        };
        let (r, ..) = counter(cx, policy);
        let _held = interest(cx, &r);
        r.update(cx, |r, cx| r.refresh(cx));
        let late = cx.update(|cx| Interest::new(r.clone(), cx));
        // At each change, the value and whether the late interest took it as its load.
        let seen = Rc::new(std::cell::RefCell::new(vec![]));
        let (log, due) = (seen.clone(), late.due);
        let _watch = cx.update(|cx| {
            cx.observe(late.resource(), move |r, cx| {
                let r = r.read(cx);
                log.borrow_mut().push((r.value, r.loads >= due));
            })
        });
        assert!(cx.read(|cx| late.loaded(cx).is_none()));
        settle(cx, &r);
        let seen = seen.borrow();
        assert!(
            seen.contains(&(Some(2), false)),
            "the running load landed unserving: {seen:?}"
        );
        assert!(!seen.contains(&(Some(2), true)), "{seen:?}");
        assert_eq!(
            cx.read(|cx| late.loaded(cx).map(|r| r.value)),
            Some(Some(3))
        );
    }

    #[gpui::test]
    fn refreshes_while_a_load_runs_end_in_one_more_load_after_it(cx: &mut TestAppContext) {
        let _dir = crate::boot(cx);
        let policy = Policy {
            fresh_for: None,
            poll: None,
        };
        let (r, n, _) = counter(cx, policy);
        let _held = interest(cx, &r);
        r.update(cx, |r, cx| (0..10).for_each(|_| r.refresh(cx)));
        settle(cx, &r);
        assert_eq!(
            n.load(SeqCst),
            3,
            "the first, one of the ten, then one for the other nine"
        );
        assert_eq!(value(cx, &r), Some(3));
    }

    #[gpui::test]
    fn an_edit_while_a_load_runs_drops_that_load_s_value(cx: &mut TestAppContext) {
        let _dir = crate::boot(cx);
        let policy = Policy {
            fresh_for: None,
            poll: None,
        };
        let (r, ..) = counter(cx, policy);
        let _held = interest(cx, &r);
        r.update(cx, |r, cx| {
            r.refresh(cx);
            r.edit(|v| *v = 10, cx);
        });
        settle(cx, &r);
        assert_eq!(value(cx, &r), Some(10), "the load read before the edit");
        r.update(cx, |r, cx| r.refresh(cx));
        settle(cx, &r);
        assert_eq!(value(cx, &r), Some(3));
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

    #[gpui::test]
    fn resetting_a_held_resource_discards_the_cache_and_in_flight_load(cx: &mut TestAppContext) {
        let _dir = crate::boot(cx);
        let (r, ..) = counter(
            cx,
            Policy {
                fresh_for: None,
                poll: None,
            },
        );
        let on = On::MrRemote("/r".into());
        r.update(cx, |r, _| r.on.push(on.clone()));
        let _held = interest(cx, &r);
        settle(cx, &r);
        assert_eq!(value(cx, &r), Some(1));

        let seen = Rc::new(std::cell::RefCell::new(vec![]));
        let log = seen.clone();
        let _watch =
            cx.update(|cx| cx.observe(&r, move |r, cx| log.borrow_mut().push(r.read(cx).value)));
        cx.update(|cx| {
            r.update(cx, |r, cx| r.refresh(cx));
            input(cx, Input::Reset(on));
            assert!(r.read(cx).value.is_none());
        });
        settle(cx, &r);
        assert!(!seen.borrow().contains(&Some(2)));
        assert_eq!(value(cx, &r), Some(3));
    }
}
