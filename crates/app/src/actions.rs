//! Every action of docs/KEYMAP.md; the vertical owning an action's behavior registers its handler.

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use gpui_kit::{App, Global, Window};

use kernel::activity::Activity;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    Navigate,
    Search,
    Vcs,
    Diff,
    View,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Action {
    pub id: &'static str,
    pub title: &'static str,
    pub group: Group,
}

const fn a(id: &'static str, title: &'static str, group: Group) -> Action {
    Action { id, title, group }
}

/// Append-only. The palette and menus list unregistered actions too, by these titles.
pub const ACTIONS: &[Action] = &[
    a("search.everywhere", "Search Everywhere", Group::Search),
    a("file.goto", "Go to File", Group::Search),
    a("find.inFiles", "Find in Files", Group::Search),
    a("action.find", "Find Action", Group::Search),
    a("file.recent", "Recent Files", Group::Navigate),
    a("editor.gotoLine", "Go to Line", Group::Navigate),
    a("nav.back", "Back", Group::Navigate),
    a("nav.forward", "Forward", Group::Navigate),
    a("panel.project", "Project Tree", Group::View),
    a("vcs.update", "Update Project (Pull)", Group::Vcs),
    a("vcs.popup", "VCS Operations", Group::Vcs),
    a("vcs.fetch", "Fetch", Group::Vcs),
    a("vcs.fetchAll", "Fetch All Repositories", Group::Vcs),
    a("vcs.pullAll", "Pull All Repositories", Group::Vcs),
    a("vcs.checkout", "Checkout Branch…", Group::Vcs),
    a("vcs.newBranch", "New Branch…", Group::Vcs),
    a("vcs.deleteBranch", "Delete Branch…", Group::Vcs),
    a("vcs.prune", "Prune Remote Branches", Group::Vcs),
    a("diff.show", "Show Diff", Group::Diff),
    a("diff.next", "Next Change", Group::Diff),
    a("diff.prev", "Previous Change", Group::Diff),
    a("file.openInZed", "Open in Zed", Group::Navigate),
    a("app.settings", "Settings…", Group::View),
    a("panel.log", "Git Log", Group::View),
    a("panel.mergeRequests", "Merge Requests", Group::View),
    a("panel.pipelines", "Pipelines", Group::View),
    a("vcs.fileHistory", "Show History for File", Group::Vcs),
    a("mr.create", "Create Merge Request…", Group::Vcs),
    a("pipeline.run", "Run Pipeline…", Group::Vcs),
    a("markdown.preview", "Toggle Markdown Preview", Group::View),
    a("panel.terminal", "Terminal", Group::View),
    a("agent.chat", "Claude", Group::View),
    a("zone.left", "Left Zone", Group::View),
    a("zone.bottom", "Bottom Zone", Group::View),
    a("zone.right", "Right Zone", Group::View),
];

pub fn find(id: &str) -> Option<&'static Action> {
    ACTIONS.iter().find(|a| a.id == id)
}

/// What key bindings and menu items carry: the action id to dispatch.
#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = detached, no_json)]
pub struct Run(pub &'static str);

type Handler = Rc<dyn Fn(&mut Window, &mut App)>;

#[derive(Default)]
struct Handlers {
    last: u64,
    /// Latest last; the latest registration of an id is the one that runs.
    stack: Vec<(Action, u64, Handler)>,
}

#[derive(Default)]
struct Registry(Rc<RefCell<Handlers>>);

impl Global for Registry {}

/// Dropping it removes its handler, so an earlier registration of the id runs again.
#[must_use]
pub struct Registration {
    key: u64,
    handlers: Weak<RefCell<Handlers>>,
}

impl Drop for Registration {
    fn drop(&mut self) {
        if let Some(h) = self.handlers.upgrade() {
            h.borrow_mut().stack.retain(|(_, key, _)| *key != self.key);
        }
    }
}

/// Panics on an id missing from `ACTIONS`: a typo, caught by the first run.
pub fn register(
    cx: &mut App,
    id: &'static str,
    title: &'static str,
    group: Group,
    handler: impl Fn(&mut Window, &mut App) + 'static,
) -> Registration {
    assert!(find(id).is_some(), "unknown action {id}");
    let handlers = cx.default_global::<Registry>().0.clone();
    let mut h = handlers.borrow_mut();
    h.last += 1;
    let key = h.last;
    h.stack
        .push((Action { id, title, group }, key, Rc::new(handler)));
    Registration {
        key,
        handlers: Rc::downgrade(&handlers),
    }
}

/// Puts `regs` over every other registration of their ids, as for the tab that came on screen.
pub fn raise(regs: &[Registration], cx: &App) {
    let Some(r) = cx.try_global::<Registry>() else {
        return;
    };
    let mut h = r.0.borrow_mut();
    for reg in regs {
        if let Some(i) = h.stack.iter().position(|(_, key, _)| *key == reg.key) {
            let entry = h.stack.remove(i);
            h.stack.push(entry);
        }
    }
}

fn registered(cx: &App, id: &str) -> Option<(Action, Handler)> {
    let h = cx.try_global::<Registry>()?.0.borrow();
    let (a, _, f) = h.stack.iter().rev().find(|(a, ..)| a.id == id)?;
    Some((*a, f.clone()))
}

/// Every action in list order, as its latest registration titles it, and whether one is registered.
pub fn all(cx: &App) -> Vec<(Action, bool)> {
    let shown = |a: &Action| registered(cx, a.id).map_or((*a, false), |(r, _)| (r, true));
    ACTIONS.iter().map(shown).collect()
}

/// Whether a handler ran.
pub fn run(window: &mut Window, cx: &mut App, id: &str) -> bool {
    registered(cx, id).map(|(_, f)| f(window, cx)).is_some()
}

/// Runs `id`, or says in the status bar that it is not available.
pub fn dispatch(window: &mut Window, cx: &mut App, id: &str) {
    if !run(window, cx, id) {
        let title = find(id).map_or(id, |a| a.title);
        let text = format!("{title} is not available yet");
        Activity::global(cx).update(cx, |a, cx| a.say(text, false, cx));
    }
}

/// Lets key bindings and menu items reach `dispatch` through `Run`.
pub fn init(cx: &mut App) {
    cx.on_action(|a: &Run, cx| {
        let id = a.0;
        // No window is active in headless tests; the app has one window.
        if let Some(window) = cx.active_window().or_else(|| cx.windows().first().copied()) {
            // The window is still dispatching the key: run once it is free.
            cx.defer(move |cx| {
                window
                    .update(cx, |_, window, cx| dispatch(window, cx, id))
                    .ok();
            });
        }
    });
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use gpui_kit::TestAppContext;

    use super::*;

    fn reg(
        cx: &mut App,
        id: &'static str,
        f: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Registration {
        register(cx, id, "Title", Group::View, f)
    }

    fn counter(n: &Rc<Cell<u32>>) -> impl Fn(&mut Window, &mut App) + 'static {
        let n = n.clone();
        move |_, _| n.set(n.get() + 1)
    }

    #[gpui_kit::test]
    fn runs_a_registered_action_and_unregisters_it(cx: &mut TestAppContext) {
        let ran = Rc::new(Cell::new(0));
        let cx = cx.add_empty_window();
        let off = cx.update(|_, cx| reg(cx, "file.goto", counter(&ran)));
        assert!(cx.update(|w, cx| run(w, cx, "file.goto")));
        drop(off);
        assert!(!cx.update(|w, cx| run(w, cx, "file.goto")));
        assert_eq!(ran.get(), 1);
    }

    #[gpui_kit::test]
    fn the_latest_registration_wins_and_dropping_it_restores_the_previous(cx: &mut TestAppContext) {
        let (main, mr) = (Rc::new(Cell::new(0)), Rc::new(Cell::new(0)));
        let cx = cx.add_empty_window();
        let _main = cx.update(|_, cx| reg(cx, "diff.next", counter(&main)));
        let over = cx.update(|_, cx| reg(cx, "diff.next", counter(&mr)));
        cx.update(|w, cx| run(w, cx, "diff.next"));
        drop(over);
        cx.update(|w, cx| run(w, cx, "diff.next"));
        assert_eq!((main.get(), mr.get()), (1, 1));
    }

    #[gpui_kit::test]
    fn a_raised_registration_wins_again(cx: &mut TestAppContext) {
        let (a, b) = (Rc::new(Cell::new(0)), Rc::new(Cell::new(0)));
        let cx = cx.add_empty_window();
        let first = cx.update(|_, cx| reg(cx, "diff.next", counter(&a)));
        let _second = cx.update(|_, cx| reg(cx, "diff.next", counter(&b)));
        cx.update(|w, cx| {
            raise(std::slice::from_ref(&first), cx);
            run(w, cx, "diff.next")
        });
        assert_eq!((a.get(), b.get()), (1, 0));
    }

    #[gpui_kit::test]
    fn an_earlier_registration_dropped_first_stays_gone(cx: &mut TestAppContext) {
        let (a, b) = (Rc::new(Cell::new(0)), Rc::new(Cell::new(0)));
        let cx = cx.add_empty_window();
        let first = cx.update(|_, cx| reg(cx, "nav.back", counter(&a)));
        let second = cx.update(|_, cx| reg(cx, "nav.back", counter(&b)));
        drop(first);
        cx.update(|w, cx| run(w, cx, "nav.back"));
        drop(second);
        assert!(!cx.update(|w, cx| run(w, cx, "nav.back")));
        assert_eq!((a.get(), b.get()), (0, 1));
    }

    #[gpui_kit::test]
    #[should_panic(expected = "unknown action nope")]
    fn rejects_unknown_ids(cx: &mut TestAppContext) {
        assert!(find("search.everywhere").is_some());
        cx.update(|cx| drop(reg(cx, "nope", |_, _| {})));
    }

    #[gpui_kit::test]
    fn lists_every_action_by_its_registered_title(cx: &mut TestAppContext) {
        let _r =
            cx.update(|cx| register(cx, "vcs.fetch", "Fetch Repo", Group::Navigate, |_, _| {}));
        let all = cx.update(|cx| all(cx));
        assert_eq!(all.len(), ACTIONS.len());
        let get = |id| *all.iter().find(|(a, _)| a.id == id).unwrap();
        assert_eq!(
            get("vcs.fetch"),
            (a("vcs.fetch", "Fetch Repo", Group::Navigate), true)
        );
        assert_eq!(get("vcs.prune"), (*find("vcs.prune").unwrap(), false));
    }

    #[test]
    fn ids_are_unique() {
        for (i, a) in ACTIONS.iter().enumerate() {
            assert!(ACTIONS[..i].iter().all(|b| b.id != a.id), "{}", a.id);
        }
    }
}
