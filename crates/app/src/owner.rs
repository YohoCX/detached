//! The owner socket's end in the app: `detached-cli` in a terminal tab reads with the owner's view
//! and asks for `env run`, once per tab.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc as reply;

use detached_core::error::{Error, Result};
use detached_core::os;
use detached_core::owner::{self, Ask, Server, Tabs};
use futures::StreamExt;
use futures::channel::mpsc;
use gpui_kit::{App, Global, PromptLevel, Task, Window};
use kernel::activity::Activity;
use kernel::blocking;
use serde_json::Value;

struct Owner {
    socket: PathBuf,
    tabs: Arc<Tabs>,
    /// Tabs the owner let `env run` in.
    allowed: HashSet<u64>,
    _server: Option<Server>,
}

impl Global for Owner {}

type Request = (u64, Ask, reply::Sender<Result<Value>>);

/// Opens the socket; `query` answers `detached-cli q`. Resolves once it listens.
pub fn start(
    query: impl Fn(&str, &mut App) -> Task<Result<Value>> + 'static,
    cx: &mut App,
) -> Task<Result<()>> {
    let socket = owner::socket(&kernel::core(cx).data_dir);
    let tabs = Arc::new(Tabs::default());
    let (tx, mut rx) = mpsc::unbounded::<Request>();
    let closing = || Error::Other("Detached is closing.".into());
    let handle: owner::Handler = Box::new(move |tab, ask| {
        let (send, answer) = reply::channel();
        tx.unbounded_send((tab, ask, send)).map_err(|_| closing())?;
        answer.recv().map_err(|_| closing())?
    });
    cx.set_global(Owner {
        socket: socket.clone(),
        tabs: tabs.clone(),
        allowed: HashSet::new(),
        _server: None,
    });
    cx.spawn(async move |cx| {
        while let Some((tab, ask, send)) = rx.next().await {
            let answer = cx.update(|cx| match ask {
                Ask::Q(q) => query(&q, cx),
                Ask::Env(profile) => env(tab, profile, cx),
            });
            cx.spawn(async move |_| send.send(answer.await).ok())
                .detach();
        }
    })
    .detach();
    let serving = blocking::run(cx, move || owner::serve(&socket, tabs, handle));
    cx.spawn(async move |cx| {
        let server = serving.await.inspect_err(|e| {
            let e = format!("detached-cli cannot reach Detached: {e}");
            cx.update(|cx| Activity::global(cx).update(cx, |a, cx| a.say(e, true, cx)));
        })?;
        cx.update_global::<Owner, _>(|o, _| o._server = Some(server));
        Ok(())
    })
}

/// The socket a new shell is told, once `start` ran.
pub fn socket(cx: &App) -> Option<PathBuf> {
    cx.try_global::<Owner>().map(|o| o.socket.clone())
}

/// Lets the processes of terminal tab `id` connect while `shell` lives.
pub fn add_tab(id: u64, shell: &Arc<os::Session>, cx: &App) {
    if let Some(o) = cx.try_global::<Owner>() {
        o.tabs.add(id, shell);
    }
}

fn env(tab: u64, profile: String, cx: &mut App) -> Task<Result<Value>> {
    let window = cx.active_window().or_else(|| cx.windows().first().copied());
    let prompt = (!cx.global::<Owner>().allowed.contains(&tab)).then(|| {
        let msg = format!("A terminal tab asks for the env profile \"{profile}\".");
        let detail = Some("Allow `detached-cli env run` in this tab until it closes?");
        let answers = ["Allow", "Deny"];
        let prompt = |_, w: &mut Window, cx: &mut App| {
            w.prompt(PromptLevel::Warning, &msg, detail, &answers, cx)
        };
        window?.update(cx, prompt).ok()
    });
    cx.spawn(async move |cx| {
        if let Some(prompt) = prompt {
            let answer = prompt.ok_or_else(|| Error::Other("No window to ask in.".into()))?;
            if answer.await != Ok(0) {
                return Err(Error::Other("The owner did not allow it.".into()));
            }
            cx.update_global::<Owner, _>(|o, _| o.allowed.insert(tab));
        }
        let e = format!("No env store yet: profile \"{profile}\" cannot be read.");
        Err(Error::Other(e))
    })
}
