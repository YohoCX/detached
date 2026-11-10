//! A view's last result, kept for the session, so a reopened view shows it while it refreshes.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use detached_core::error::Result;
use gpui_kit::{App, BorrowAppContext, Global, Task};

use kernel::blocking;

/// A focus refresh this soon after the last one ended is skipped.
const FRESH: Duration = Duration::from_secs(30);

pub struct Cached<T> {
    pub value: Option<T>,
    /// The latest refresh's failure; the value stays.
    pub error: Option<String>,
    /// When the latest refresh ended.
    pub at: Instant,
}

/// One per value type, keyed by the view's own key (repo, filter, ...).
pub struct Cache<T>(HashMap<String, Cached<T>>);

impl<T> Default for Cache<T> {
    fn default() -> Self {
        Cache(HashMap::new())
    }
}

impl<T: 'static> Global for Cache<T> {}

impl<T> Cache<T> {
    pub fn get(&self, key: &str) -> Option<&Cached<T>> {
        self.0.get(key)
    }

    fn due(&self, key: &str, focus: bool, now: Instant) -> bool {
        !focus
            || self
                .0
                .get(key)
                .is_none_or(|c| now.duration_since(c.at) >= FRESH)
    }

    fn put(&mut self, key: String, r: std::result::Result<T, String>, now: Instant) {
        let c = self.0.entry(key).or_insert(Cached {
            value: None,
            error: None,
            at: now,
        });
        c.at = now;
        match r {
            Ok(v) => (c.value, c.error) = (Some(v), None),
            Err(e) => c.error = Some(e),
        }
    }
}

pub fn cached<'a, T: 'static>(key: &str, cx: &'a App) -> Option<&'a Cached<T>> {
    cx.try_global::<Cache<T>>()?.get(key)
}

/// Runs `f` on the pool into the cache under `key` (observers of `Cache<T>` are notified).
/// Resolves to the new value; None when it failed, or when `focus` is set and the last refresh ended
/// less than 30 s ago, which skips it.
pub fn refresh<T: Clone + Send + 'static>(
    key: impl Into<String>,
    focus: bool,
    f: impl FnOnce() -> Result<T> + Send + 'static,
    cx: &mut App,
) -> Task<Option<T>> {
    let key = key.into();
    let now = cx.background_executor().now();
    if !cx.default_global::<Cache<T>>().due(&key, focus, now) {
        return Task::ready(None);
    }
    let r = blocking::run(cx, move || f().map_err(|e| e.to_string()));
    cx.spawn(async move |cx| {
        let r = r.await;
        cx.update(|cx| {
            let value = r.as_ref().ok().cloned();
            let now = cx.background_executor().now();
            cx.update_global::<Cache<T>, _>(|c, _| c.put(key, r, now));
            value
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_last_value_on_failure_and_skips_a_focus_refresh_within_30_s() {
        let mut c = Cache::default();
        let t = Instant::now();
        assert!(c.due("k", false, t));
        c.put("k".into(), Ok(1), t);
        assert!(!c.due("k", true, t + Duration::from_secs(29)));
        assert!(c.due("k", false, t));
        c.put("k".into(), Err("down".into()), t);
        let k = c.get("k").unwrap();
        assert_eq!((k.value, k.error.as_deref()), (Some(1), Some("down")));
        c.put("k".into(), Ok(2), t);
        assert_eq!(c.get("k").unwrap().error, None);
        assert!(c.due("k", true, t + FRESH));
    }

    #[gpui_kit::test]
    fn refreshes_through_the_pool(cx: &mut gpui_kit::TestAppContext) {
        let task = cx.update(|cx| refresh("k", false, || Ok(1), cx));
        cx.executor().allow_parking();
        assert_eq!(cx.foreground_executor().block_test(task), Some(1));
        let skipped = cx.update(|cx| refresh("k", true, || Ok(2), cx));
        assert_eq!(cx.foreground_executor().block_test(skipped), None);
        cx.update(|cx| assert_eq!(cached::<i32>("k", cx).unwrap().value, Some(1)));
    }
}
