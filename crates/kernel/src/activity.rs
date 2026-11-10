//! What the app is doing and has said: running ops, notices, the status message, and the log of
//! errors and warnings the Events app shows.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use detached_core::state::AppState;
use gpui::{App, AppContext, Context, Entity, EventEmitter, Global, Task};

use crate::blocking;

/// A running network op, shown in the status bar with its latest progress line.
#[derive(Clone, Debug, PartialEq)]
pub struct Op {
    pub id: u64,
    pub repo: String,
    pub label: String,
    pub line: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Level {
    #[default]
    Info,
    Warn,
    Error,
}

/// `repo` and `files` list files with an Open in Zed button (checkout conflicts).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Notice {
    pub id: u64,
    pub level: Level,
    pub title: String,
    pub body: Option<String>,
    pub repo: Option<String>,
    pub files: Vec<String>,
}

/// An error or a warning, kept for the Events app.
#[derive(Clone, Debug, PartialEq)]
pub struct Logged {
    /// Unix seconds.
    pub at: i64,
    pub level: Level,
    pub text: String,
}

/// What the Events app keeps at most.
const LOG: usize = 500;

#[derive(Clone, Debug, PartialEq)]
pub struct Message {
    pub text: String,
    pub error: bool,
}

/// An op on `repo` ended.
pub struct OpEnded {
    pub repo: String,
}

#[derive(Default)]
pub struct Activity {
    pub ops: Vec<Op>,
    pub notices: Vec<Notice>,
    pub message: Option<Message>,
    /// Errors and warnings, oldest first.
    pub log: Vec<Logged>,
    /// Log entries since the Events app was last on screen.
    pub unread: usize,
    last_notice: u64,
}

struct GlobalActivity(Entity<Activity>);

impl Global for GlobalActivity {}

impl EventEmitter<OpEnded> for Activity {}

pub(crate) fn init(cx: &mut App) {
    let activity = cx.new(|_| Activity::default());
    cx.set_global(GlobalActivity(activity));
}

impl Activity {
    pub fn global(cx: &App) -> Entity<Activity> {
        cx.global::<GlobalActivity>().0.clone()
    }

    /// The single status message slot, for 10 s or until replaced or clicked away; an error stays
    /// in the log.
    pub fn say(&mut self, text: impl Into<String>, error: bool, cx: &mut Context<Self>) {
        let text = text.into();
        if error {
            self.record(Level::Error, text.clone(), cx);
        }
        let said = Message { text, error };
        self.message = Some(said.clone());
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_secs(10))
                .await;
            this.update(cx, |a, cx| {
                if a.message.as_ref() == Some(&said) {
                    a.clear_message(cx);
                }
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// Keeps an error or warning for the Events app.
    pub fn record(&mut self, level: Level, text: impl Into<String>, cx: &mut Context<Self>) {
        let at = SystemTime::now().duration_since(UNIX_EPOCH);
        self.log.push(Logged {
            at: at.map_or(0, |d| d.as_secs() as i64),
            level,
            text: text.into(),
        });
        if self.log.len() > LOG {
            self.log.remove(0);
        }
        self.unread += 1;
        cx.notify();
    }

    pub fn read_log(&mut self, cx: &mut Context<Self>) {
        if self.unread > 0 {
            self.unread = 0;
            cx.notify();
        }
    }

    pub fn clear_log(&mut self, cx: &mut Context<Self>) {
        self.log.clear();
        self.unread = 0;
        cx.notify();
    }

    pub fn clear_message(&mut self, cx: &mut Context<Self>) {
        self.message = None;
        cx.notify();
    }

    /// Adds `n` with a fresh id; info notices go away after 6 s.
    pub fn notify(&mut self, mut n: Notice, cx: &mut Context<Self>) -> u64 {
        self.last_notice += 1;
        let id = self.last_notice;
        n.id = id;
        if n.level != Level::Info {
            let text = match &n.body {
                Some(body) => format!("{}\n{body}", n.title),
                None => n.title.clone(),
            };
            self.record(n.level, text, cx);
        }
        if n.level == Level::Info {
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(Duration::from_secs(6)).await;
                this.update(cx, |a, cx| a.dismiss(id, cx)).ok();
            })
            .detach();
        }
        self.notices.push(n);
        cx.notify();
        id
    }

    pub fn dismiss(&mut self, id: u64, cx: &mut Context<Self>) {
        self.notices.retain(|n| n.id != id);
        cx.notify();
    }

    /// Runs a cancellable op (`f` gets the op id and a progress sink) with its latest progress line in
    /// the status bar, then emits `OpEnded`.
    pub fn run_op<R: Send + 'static>(
        &mut self,
        repo: &str,
        label: &str,
        f: impl FnOnce(&AppState, u64, &(dyn Fn(&str) + Sync)) -> R + Send + 'static,
        cx: &mut Context<Self>,
    ) -> Task<R> {
        let core = crate::core(cx).clone();
        let id = core.ops.id();
        self.ops.push(Op {
            id,
            repo: repo.into(),
            label: label.into(),
            line: String::new(),
        });
        cx.notify();
        let weak = cx.weak_entity();
        let done = blocking::stream(
            cx,
            move |send| f(&core, id, &|line: &str| send(line.to_string())),
            move |mut lines: Vec<String>, cx| {
                weak.update(cx, |a, cx| {
                    if let (Some(op), Some(line)) =
                        (a.ops.iter_mut().find(|o| o.id == id), lines.pop())
                    {
                        op.line = line;
                        cx.notify();
                    }
                })
                .ok();
            },
        );
        let repo = repo.to_string();
        cx.spawn(async move |this, cx| {
            let r = done.await;
            this.update(cx, |a, cx| {
                a.ops.retain(|o| o.id != id);
                cx.emit(OpEnded { repo });
                cx.notify();
            })
            .ok();
            r
        })
    }

    pub fn cancel_op(id: u64, cx: &App) {
        crate::core(cx).ops.cancel(id);
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::atomic::Ordering;

    use gpui::TestAppContext;

    use super::*;

    fn boot(cx: &mut TestAppContext) -> (tempfile::TempDir, Entity<Activity>) {
        let dir = crate::boot(cx);
        (dir, cx.update(|cx| Activity::global(cx)))
    }

    fn record(n: usize, activity: &Entity<Activity>, cx: &mut TestAppContext) {
        activity.update(cx, |a, cx| {
            (0..n).for_each(|i| a.record(Level::Warn, i.to_string(), cx))
        });
    }

    #[gpui::test]
    fn an_op_is_recorded_while_it_runs_and_removed_when_it_ends(cx: &mut TestAppContext) {
        let (_dir, activity) = boot(cx);
        let ended = Rc::new(RefCell::new(vec![]));
        let seen = ended.clone();
        let _sub = cx.update(|cx| {
            cx.subscribe(&activity, move |_, e: &OpEnded, _| {
                seen.borrow_mut().push(e.repo.clone())
            })
        });
        let (go, wait) = std::sync::mpsc::channel::<()>();
        let task = activity.update(cx, |a, cx| {
            a.run_op(
                "/r",
                "Fetch",
                move |_, id, _| {
                    wait.recv().unwrap();
                    id
                },
                cx,
            )
        });
        let op = activity.read_with(cx, |a, _| a.ops.clone());
        assert_eq!((op.len(), op[0].repo.as_str()), (1, "/r"));
        go.send(()).unwrap();
        cx.executor().allow_parking();
        assert_eq!(cx.foreground_executor().block_test(task), op[0].id);
        assert!(activity.read_with(cx, |a, _| a.ops.is_empty()));
        assert_eq!(*ended.borrow(), ["/r"]);
    }

    #[gpui::test]
    fn a_cancel_reaches_the_running_op_and_it_stops(cx: &mut TestAppContext) {
        let (_dir, activity) = boot(cx);
        let task = activity.update(cx, |a, cx| {
            a.run_op(
                "/r",
                "Fetch",
                |core, id, _| {
                    core.op(id, |cancel| {
                        while !cancel.load(Ordering::SeqCst) {
                            std::thread::yield_now();
                        }
                        "cancelled"
                    })
                },
                cx,
            )
        });
        let id = activity.read_with(cx, |a, _| a.ops[0].id);
        cx.update(|cx| Activity::cancel_op(id, cx));
        cx.executor().allow_parking();
        assert_eq!(cx.foreground_executor().block_test(task), "cancelled");
        assert!(activity.read_with(cx, |a, _| a.ops.is_empty()));
    }

    #[gpui::test]
    fn info_notices_go_after_6_s_and_others_stay(cx: &mut TestAppContext) {
        let (_dir, activity) = boot(cx);
        let notice = |level| Notice {
            level,
            title: "t".into(),
            ..Notice::default()
        };
        let (info, warn) = activity.update(cx, |a, cx| {
            (
                a.notify(notice(Level::Info), cx),
                a.notify(notice(Level::Warn), cx),
            )
        });
        assert_ne!(info, warn);
        cx.executor().advance_clock(Duration::from_secs(5));
        assert_eq!(activity.read_with(cx, |a, _| a.notices.len()), 2);
        cx.executor().advance_clock(Duration::from_secs(1));
        let ids = activity.read_with(cx, |a, _| {
            a.notices.iter().map(|n| n.id).collect::<Vec<_>>()
        });
        assert_eq!(ids, [warn]);
    }

    #[gpui::test]
    fn the_log_keeps_500_and_drops_the_oldest(cx: &mut TestAppContext) {
        let (_dir, activity) = boot(cx);
        record(LOG + 1, &activity, cx);
        activity.read_with(cx, |a, _| {
            assert_eq!(a.log.len(), LOG);
            assert_eq!(a.log[0].text, "1");
            assert_eq!(a.log[LOG - 1].text, LOG.to_string());
        });
    }

    #[gpui::test]
    fn unread_counts_and_clears(cx: &mut TestAppContext) {
        let (_dir, activity) = boot(cx);
        record(2, &activity, cx);
        activity.update(cx, |a, cx| a.say("failed", true, cx));
        assert_eq!(activity.read_with(cx, |a, _| a.unread), 3);
        activity.update(cx, |a, cx| a.read_log(cx));
        assert_eq!(
            activity.read_with(cx, |a, _| (a.unread, a.log.len())),
            (0, 3)
        );
        record(1, &activity, cx);
        assert_eq!(activity.read_with(cx, |a, _| a.unread), 1);
        activity.update(cx, |a, cx| a.clear_log(cx));
        assert_eq!(
            activity.read_with(cx, |a, _| (a.unread, a.log.len())),
            (0, 0)
        );
    }
}
