//! The owner socket's end in the app: `detached-cli` in a terminal tab reads with the owner's view
//! and asks for `env run`, once per tab.

use std::collections::HashMap;
use std::io::ErrorKind;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc as reply;

use detached_core::error::{Error, Result};
use detached_core::os;
use detached_core::owner::{self, Ask, Server, Tabs};
use detached_core::secrets;
use futures::channel::mpsc;
use futures::future::Shared;
use futures::{FutureExt, StreamExt};
use gpui_kit::{App, Global, PromptLevel, Task, Window};
use kernel::activity::Activity;
use kernel::blocking;
use serde_json::Value;

struct Owner {
    socket: PathBuf,
    tabs: Arc<Tabs>,
    /// Each tab's answer to `env run`, asked once and shared while it waits.
    allowed: HashMap<u64, Shared<Task<std::result::Result<(), &'static str>>>>,
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
        allowed: HashMap::new(),
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
            let message = match e {
                Error::Io(err) if err.kind() == ErrorKind::AddrInUse => String::from(
                    "The detached-cli connection for this data folder is already in use. If another \
                     Detached instance is using this folder, quit it and relaunch this instance.",
                ),
                #[cfg(windows)]
                Error::Io(err) if err.kind() == ErrorKind::PermissionDenied => format!(
                    "This instance could not start detached-cli support ({e}). Check whether another \
                     Detached instance is using this data folder or whether access is denied."
                ),
                _ => format!("This instance could not start detached-cli support ({e})."),
            };
            cx.update(|cx| Activity::global(cx).update(cx, |a, cx| a.say(message, true, cx)));
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
    let allowed = match cx.global::<Owner>().allowed.get(&tab) {
        Some(allowed) => allowed.clone(),
        None => {
            let allowed = ask(tab, &profile, cx).shared();
            cx.global_mut::<Owner>()
                .allowed
                .insert(tab, allowed.clone());
            allowed
        }
    };
    let core = kernel::core(cx).clone();
    cx.spawn(async move |cx| {
        allowed.await.map_err(|e| Error::Other(e.into()))?;
        let values = cx.update(|cx| blocking::run(cx, move || secrets::values(&core, &profile)));
        let values = values
            .await?
            .into_iter()
            .map(|(k, v)| (k, Value::String(v)));
        Ok(Value::Object(values.collect()))
    })
}

/// Asks the owner to allow `env run` in `tab`; a refusal is forgotten, so the next run asks again.
fn ask(tab: u64, profile: &str, cx: &mut App) -> Task<std::result::Result<(), &'static str>> {
    let window = cx.active_window().or_else(|| cx.windows().first().copied());
    let msg = format!("A terminal tab asks for the env profile \"{profile}\".");
    let detail = Some("Allow `detached-cli env run` in this tab until it closes?");
    let prompt = |_, w: &mut Window, cx: &mut App| {
        w.prompt(PromptLevel::Warning, &msg, detail, &["Allow", "Deny"], cx)
    };
    let prompt = window.and_then(|w| w.update(cx, prompt).ok());
    cx.spawn(async move |cx| {
        let answer = match prompt {
            None => Err("No window to ask in."),
            Some(p) => match p.await {
                Ok(0) => Ok(()),
                _ => Err("The owner did not allow it."),
            },
        };
        if answer.is_err() {
            cx.update_global::<Owner, _>(|o, _| o.allowed.remove(&tab));
        }
        answer
    })
}
