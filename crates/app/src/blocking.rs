//! Core blocks, some calls for minutes, so it runs on a pool of plain threads, never on GPUI's executors.

use std::panic::{self, AssertUnwindSafe};
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::thread;
use std::time::Duration;

use futures::StreamExt;
use futures::channel::{mpsc as chan, oneshot};
use gpui_kit::{App, Task};

type Job = Box<dyn FnOnce() + Send>;

/// Enough for fetch all's parallelism next to statuses, logs and finds.
const THREADS: usize = 16;
pub const FRAME: Duration = Duration::from_millis(16);

fn submit(job: Job) {
    static POOL: OnceLock<mpsc::Sender<Job>> = OnceLock::new();
    let pool = POOL.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<Job>();
        let rx = Arc::new(Mutex::new(rx));
        for i in 0..THREADS {
            let rx = rx.clone();
            thread::Builder::new()
                .name(format!("core-{i}"))
                .spawn(move || {
                    loop {
                        // Its own statement, so the lock is released before the job runs.
                        let job = rx.lock().unwrap().recv();
                        match job {
                            Ok(job) => job(),
                            Err(_) => return,
                        }
                    }
                })
                .expect("cannot start the core threads");
        }
        tx
    });
    // The pool lives as long as the process, so sending cannot fail.
    let _ = pool.send(job);
}

/// Runs `f` on the pool; the task resolves on the UI thread with its result. A panic in `f` resumes there.
pub fn run<R: Send + 'static>(cx: &App, f: impl FnOnce() -> R + Send + 'static) -> Task<R> {
    let (tx, rx) = oneshot::channel();
    submit(Box::new(move || {
        let _ = tx.send(panic::catch_unwind(AssertUnwindSafe(f)));
    }));
    cx.foreground_executor().spawn(async move {
        match rx.await {
            Ok(Ok(r)) => r,
            Ok(Err(p)) => panic::resume_unwind(p),
            Err(_) => unreachable!("a job always answers"),
        }
    })
}

/// Runs `f` on the pool with a sink; what it sends reaches `on_batch` on the UI thread, at most one batch a
/// frame. The task resolves with `f`'s result after the last batch; dropping it stops the batches.
pub fn stream<T: Send + 'static, R: Send + 'static>(
    cx: &App,
    f: impl FnOnce(&(dyn Fn(T) + Sync)) -> R + Send + 'static,
    mut on_batch: impl FnMut(Vec<T>, &mut App) + 'static,
) -> Task<R> {
    let (tx, mut rx) = chan::unbounded();
    let done = run(cx, move || {
        f(&move |item| {
            let _ = tx.unbounded_send(item);
        })
    });
    cx.spawn(async move |cx| {
        while let Some(first) = rx.next().await {
            let mut batch = vec![first];
            batch.extend(std::iter::from_fn(|| rx.try_recv().ok()));
            cx.update(|cx| on_batch(batch, cx));
            cx.background_executor().timer(FRAME).await;
        }
        done.await
    })
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::time::Instant;

    use gpui_kit::TestAppContext;

    use super::*;

    /// Lets the test executor wait for the pool's real threads.
    fn finish<R: 'static>(cx: &mut TestAppContext, task: Task<R>) -> R {
        cx.executor().allow_parking();
        cx.foreground_executor().block_test(task)
    }

    #[gpui_kit::test]
    fn runs_off_the_ui_thread_and_resumes_on_it(cx: &mut TestAppContext) {
        let ui = thread::current().id();
        let task = cx.update(|cx| run(cx, move || thread::current().id()));
        assert_ne!(finish(cx, task), ui);
    }

    #[gpui_kit::test]
    fn runs_sixteen_jobs_at_once(cx: &mut TestAppContext) {
        let barrier = Arc::new(std::sync::Barrier::new(THREADS));
        let tasks: Vec<_> = (0..THREADS)
            .map(|_| {
                let b = barrier.clone();
                cx.update(|cx| run(cx, move || b.wait().is_leader()))
            })
            .collect();
        let leaders = tasks.into_iter().map(|t| finish(cx, t)).filter(|l| *l);
        assert_eq!(leaders.count(), 1);
    }

    #[gpui_kit::test]
    fn streams_every_item_in_order_and_then_the_result(cx: &mut TestAppContext) {
        let got = Rc::new(RefCell::new(vec![]));
        let sink = got.clone();
        let start = Instant::now();
        let task = cx.update(|cx| {
            stream(
                cx,
                |send| {
                    (0..1000).for_each(send);
                    "done"
                },
                move |batch, _| sink.borrow_mut().push(batch),
            )
        });
        assert_eq!(finish(cx, task), "done");
        let batches = got.borrow();
        let items: Vec<i32> = batches.iter().flatten().copied().collect();
        assert_eq!(items, (0..1000).collect::<Vec<_>>());
        assert!(batches.len() < 1000, "items come in batches");
        assert!(start.elapsed() < Duration::from_secs(10));
    }

    #[gpui_kit::test]
    #[should_panic(expected = "core bug")]
    fn a_panic_in_the_job_resumes_on_the_ui_thread(cx: &mut TestAppContext) {
        let task = cx.update(|cx| run(cx, || -> () { panic!("core bug") }));
        finish(cx, task);
    }
}
