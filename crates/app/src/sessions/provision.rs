use std::collections::HashMap;

use detached_core::git::worktree::{Cleanup, Preview};
use detached_core::sessions::{self, Candidate, Member, Request, Stage, Workset};
use detached_core::types::RepoInfo;
use gpui_kit::base::{Disableable, TestSupportExt};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Escape, InputEvent, InputState};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    AnyElement, AnyView, App, AppContext, Context, ElementId, Entity, FontWeight, Global,
    InteractiveElement, IntoElement, ParentElement, Render, Role, StatefulInteractiveElement,
    Styled, Task, WeakEntity, Window, div, px,
};
use kernel::activity::Activity;
use kernel::blocking;
use kernel::doc::Doc;
use kernel::workset::{self, Change};
use kernel::workspace::Workspace;

use crate::dock;
use crate::ui::{self, GAP, text, theme};

struct Controller(WeakEntity<Sheet>);
impl Global for Controller {}

pub fn overlay(window: &mut Window, cx: &mut App) -> AnyView {
    let sheet = cx.new(|cx| {
        let name = cx.new(|cx| InputState::new(window, cx));
        let branch = cx.new(|cx| InputState::new(window, cx));
        cx.subscribe_in(
            &name,
            window,
            |s: &mut Sheet, _, e: &InputEvent, window, cx| {
                if matches!(e, InputEvent::Change) {
                    s.name_changed(window, cx);
                }
            },
        )
        .detach();
        cx.subscribe(&branch, |s: &mut Sheet, _, e: &InputEvent, cx| {
            if matches!(e, InputEvent::Change) {
                s.preview(cx);
            }
        })
        .detach();
        let activity = Activity::global(cx);
        cx.observe(&activity, |_, _, cx| cx.notify()).detach();
        Sheet {
            name,
            branch,
            dialog: None,
            loading: None,
            op: None,
            busy: false,
            stop: false,
            error: None,
            retries: HashMap::new(),
        }
    });
    cx.set_global(Controller(sheet.downgrade()));
    cx.new(|_| Overlay(sheet)).into()
}

struct Overlay(Entity<Sheet>);

impl Render for Overlay {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div().when(!self.0.read(cx).page_mode(), |d| d.child(self.0.clone()))
    }
}

pub(crate) fn page(cx: &App) -> Option<AnyView> {
    let sheet = sheet(cx)?;
    sheet.read(cx).page_mode().then(|| sheet.into())
}

pub(super) fn provisional(cx: &App) -> Option<String> {
    let sheet = sheet(cx)?;
    let s = sheet.read(cx);
    if s.page_mode() {
        match &s.dialog {
            Some(Dialog::Form(_)) => {
                let value = s.name.read(cx).value();
                return Some(if value.is_empty() {
                    "New session".into()
                } else {
                    value.to_string()
                });
            }
            Some(Dialog::Result(_, set, _)) => return Some(set.name.clone()),
            _ => {}
        }
    }
    None
}

fn sheet(cx: &App) -> Option<Entity<Sheet>> {
    cx.try_global::<Controller>().and_then(|c| c.0.upgrade())
}

pub fn new(window: &mut Window, cx: &mut App) {
    form(None, window, cx);
}

pub fn add(window: &mut Window, cx: &mut App) {
    if let Some(set) = selected(cx) {
        form(Some(set), window, cx);
    }
}

fn selected(cx: &App) -> Option<Workset> {
    let w = Workspace::get(cx);
    (!w.archive())
        .then(|| {
            w.sessions()
                .iter()
                .find(|set| set.id == w.session_id())
                .cloned()
        })
        .flatten()
}

fn form(set: Option<Workset>, window: &mut Window, cx: &mut App) {
    let w = Workspace::get(cx);
    let Some(ws) = w.id().map(String::from) else {
        return;
    };
    let repos: Vec<_> = w
        .repos()
        .iter()
        .filter(|r| {
            r.worktree_of.is_none()
                && !set
                    .as_ref()
                    .is_some_and(|s| s.members.iter().any(|m| m.repo == r.path))
        })
        .cloned()
        .collect();
    if let Some(sheet) = sheet(cx) {
        sheet.update(cx, |s, cx| {
            if s.busy {
                return;
            }
            s.dialog = None;
            let branch = set.as_ref().and_then(Workset::default_branch).unwrap_or("");
            s.branch
                .update(cx, |input, cx| input.set_value(branch, window, cx));
            s.name.update(cx, |input, cx| {
                input.set_value("", window, cx);
                input.focus(window, cx);
            });
            let rows = repos
                .into_iter()
                .enumerate()
                .map(|(i, repo)| {
                    let branch = cx.new(|cx| InputState::new(window, cx));
                    let base = cx.new(|cx| InputState::new(window, cx));
                    for state in [&branch, &base] {
                        cx.subscribe(state, |s: &mut Sheet, _, e: &InputEvent, cx| {
                            if matches!(e, InputEvent::Change) {
                                s.preview(cx);
                            }
                        })
                        .detach();
                    }
                    Row {
                        repo,
                        i,
                        branch,
                        base,
                        checked: false,
                        preview: None,
                        adopt: false,
                    }
                })
                .collect();
            if set.is_some() {
                s.branch.update(cx, |input, cx| input.focus(window, cx));
            }
            s.error = None;
            s.loading = None;
            let adding = set.is_some();
            s.dialog = Some(Dialog::Form(Form {
                ws: ws.clone(),
                set,
                rows,
                generated: String::new(),
                existing: vec![],
            }));
            if adding {
                let core = kernel::core(cx).clone();
                let task = blocking::run(cx, move || sessions::candidates(&core, &ws));
                cx.spawn(async move |this, cx| {
                    let rows = task.await.unwrap_or_default();
                    _ = this.update(cx, |s, cx| {
                        if let Some(Dialog::Form(f)) = &mut s.dialog {
                            f.existing = rows;
                            cx.notify();
                        }
                    });
                })
                .detach();
            }
            cx.refresh_windows();
        });
    }
}

fn manage(set: Workset, mode: Manage, cx: &mut App) {
    let Some(ws) = Workspace::get(cx).id().map(String::from) else {
        return;
    };
    if let Some(sheet) = sheet(cx) {
        sheet.update(cx, |s, cx| {
            if s.busy {
                return;
            }
            s.loading = None;
            s.error = None;
            s.dialog = Some(Dialog::Manage(Management {
                ws,
                set,
                mode,
                rows: vec![],
                confirm: None,
            }));
            s.load_cleanup(cx);
            cx.refresh_windows();
        });
    }
}

pub fn archive(_: &mut Window, cx: &mut App) {
    if let Some(set) = selected(cx) {
        manage(set, Manage::Archive, cx);
    }
}

pub fn cleanup(set: Workset, cx: &mut App) {
    manage(set, Manage::Cleanup(None), cx);
}

pub fn remove_member(repo: String, cx: &mut App) {
    if let Some(set) = selected(cx) {
        manage(set, Manage::Cleanup(Some(repo)), cx);
    }
}

pub fn resume(set: Workset, window: &mut Window, cx: &mut App) {
    let recreate = set
        .members
        .iter()
        .any(|m| m.stage != Stage::Pending && !m.state.usable());
    manage(set, Manage::Resume(recreate), cx);
    if !recreate && let Some(sheet) = sheet(cx) {
        sheet.update(cx, |s, cx| s.commit_manage(window, cx));
    }
}

pub(super) fn recover(set: Workset, window: &mut Window, cx: &mut App) {
    if let Some((ws, sheet)) = Workspace::get(cx).id().map(String::from).zip(sheet(cx)) {
        sheet.update(cx, |s, cx| {
            if !s.busy {
                let open = set.draft;
                s.error = None;
                s.loading = None;
                s.result(ws, set, open, window, cx);
                cx.refresh_windows();
            }
        });
    }
}

type Inspection = Vec<(Member, detached_core::error::Result<Cleanup>)>;

enum Dialog {
    Form(Form),
    Manage(Management),
    Result(String, Workset, bool),
}

struct Form {
    ws: String,
    set: Option<Workset>,
    rows: Vec<Row>,
    generated: String,
    existing: Vec<Candidate>,
}

struct Row {
    repo: RepoInfo,
    i: usize,
    branch: Entity<InputState>,
    base: Entity<InputState>,
    checked: bool,
    preview: Option<detached_core::error::Result<Preview>>,
    adopt: bool,
}

impl Row {
    fn request(&self, branch: &str, cx: &App) -> Request {
        let value = self.branch.read(cx).value();
        let base = self.base.read(cx).value().trim().to_owned();
        Request {
            repo: self.repo.path.clone(),
            branch: if value.is_empty() {
                branch.into()
            } else {
                value.to_string()
            },
            base: (!base.is_empty()).then_some(base),
        }
    }
}

#[derive(Clone)]
enum Manage {
    Archive,
    Cleanup(Option<String>),
    Resume(bool),
}

struct Management {
    ws: String,
    set: Workset,
    mode: Manage,
    rows: Inspection,
    confirm: Option<String>,
}

struct Sheet {
    name: Entity<InputState>,
    branch: Entity<InputState>,
    dialog: Option<Dialog>,
    loading: Option<Task<()>>,
    op: Option<u64>,
    busy: bool,
    stop: bool,
    error: Option<String>,
    retries: HashMap<String, Entity<InputState>>,
}

impl Sheet {
    fn name_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(Dialog::Form(form)) = &mut self.dialog
            && form.set.is_none()
        {
            if self.branch.read(cx).value().as_ref() == form.generated {
                let name = self.name.read(cx).value().trim().to_lowercase();
                form.generated = if name.is_empty() {
                    String::new()
                } else {
                    format!(
                        "feature/{}",
                        name.split_whitespace().collect::<Vec<_>>().join("-")
                    )
                };
                self.branch
                    .update(cx, |input, cx| input.set_value(&form.generated, window, cx));
            }
            self.preview(cx);
            cx.refresh_windows();
        }
    }

    fn preview(&mut self, cx: &mut Context<Self>) {
        let Some(Dialog::Form(form)) = &mut self.dialog else {
            return;
        };
        if self.busy {
            return;
        }
        let branch = self.branch.read(cx).value();
        for row in form.rows.iter_mut().filter(|r| r.adopt) {
            let request = row.request(&branch, cx);
            row.adopt = form.existing.iter().any(|c| {
                c.repo == row.repo.path
                    && c.branch.as_deref() == Some(&request.branch)
                    && c.refusal.is_none()
            });
        }
        let requests: Vec<_> = form
            .rows
            .iter_mut()
            .filter(|r| r.checked && !r.adopt)
            .map(|row| {
                row.preview = None;
                (row.i, row.request(&branch, cx))
            })
            .collect();
        self.loading = None;
        if !requests.is_empty() {
            let ws = form.ws.clone();
            let core = kernel::core(cx).clone();
            let task = blocking::run(cx, move || {
                requests
                    .into_iter()
                    .map(|(i, r)| (i, sessions::preview(&core, &ws, &r)))
                    .collect::<Vec<_>>()
            });
            self.loading = Some(cx.spawn(async move |this, cx| {
                let result = task.await;
                _ = this.update(cx, |s, cx| {
                    if let Some(Dialog::Form(f)) = &mut s.dialog {
                        for (i, p) in result {
                            f.rows[i].preview = Some(p);
                        }
                    }
                    s.loading = None;
                    cx.notify();
                });
            }));
        }
        cx.notify();
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Dialog::Form(f)) = &self.dialog else {
            return;
        };
        let name = self.name.read(cx).value().trim().to_owned();
        if self.busy || !valid(f, &name) {
            return;
        }
        let ws = f.ws.clone();
        let branch = self.branch.read(cx).value().to_string();
        let selected: Vec<_> = f.rows.iter().filter(|r| r.checked).collect();
        let adopt = selected.first().filter(|r| r.adopt).and_then(|r| {
            existing(f, &r.repo.path, &r.request(&branch, cx).branch).map(|c| c.path.clone())
        });
        let requests: Vec<_> = selected.iter().map(|r| r.request(&branch, cx)).collect();
        let repos = requests.iter().map(|r| r.repo.clone()).collect();
        let existing = f.set.as_ref().map(|s| s.id.clone());
        let adding = existing.is_some();
        self.busy = true;
        self.stop = false;
        self.error = None;
        let core = kernel::core(cx).clone();
        let adopted = adopt.is_some();
        let task = blocking::run(cx, {
            let ws = ws.clone();
            move || match (existing, adopt) {
                (Some(id), Some(path)) => sessions::add_existing(&core, &ws, &id, &path),
                (Some(id), None) => sessions::begin_add(&core, &ws, &id, &requests[0]),
                _ => sessions::begin(&core, &ws, &name, &branch, &requests),
            }
        });
        cx.spawn_in(window, async move |this, cx| match task.await {
            Ok(set) if adopted => {
                _ = this.update_in(cx, |s, _, cx| {
                    save(&ws, set, false, cx);
                    s.busy = false;
                    s.close(cx);
                });
            }
            Ok(set) => {
                _ = this.update_in(cx, |s, window, cx| {
                    s.result(ws.clone(), set.clone(), !adding, window, cx);
                    s.run(ws, set, repos, Change::Provision(None), window, cx);
                });
            }
            Err(e) => _ = this.update(cx, |s, cx| s.fail(e, cx)),
        })
        .detach();
        cx.notify();
    }

    fn load_cleanup(&mut self, cx: &mut Context<Self>) {
        let Some(Dialog::Manage(m)) = &self.dialog else {
            return;
        };
        if !matches!(m.mode, Manage::Cleanup(_)) {
            return;
        }
        let (ws, id) = (m.ws.clone(), m.set.id.clone());
        let core = kernel::core(cx).clone();
        let task = blocking::run(cx, move || sessions::cleanup_preview(&core, &ws, &id));
        self.loading = Some(cx.spawn(async move |this, cx| {
            let result = task.await;
            _ = this.update(cx, |s, cx| {
                s.loading = None;
                match result {
                    Ok(rows) => {
                        if let Some(Dialog::Manage(m)) = &mut s.dialog {
                            m.rows = rows;
                        }
                    }
                    Err(e) => s.error = Some(e.to_string()),
                }
                cx.notify();
            });
        }));
    }

    fn commit_manage(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Dialog::Manage(m)) = &self.dialog else {
            return;
        };
        if self.busy {
            return;
        }
        let Management { ws, set, mode, .. } = m;
        match mode {
            Manage::Cleanup(scope) => {
                let force = m.confirm.is_some();
                let repos: Vec<_> = match &m.confirm {
                    Some(repo) => vec![repo.clone()],
                    None => m
                        .rows
                        .iter()
                        .filter(|(m, p)| {
                            scope.as_ref().is_none_or(|r| r == &m.repo)
                                && p.as_ref().is_ok_and(Cleanup::clean)
                        })
                        .map(|(m, _)| m.repo.clone())
                        .collect(),
                };
                let set = set.clone();
                if repos.is_empty() || self.refuse(&set, &repos, cx) {
                    return;
                }
                let Some(Dialog::Manage(m)) = &mut self.dialog else {
                    return;
                };
                m.confirm = None;
                let (ws, set) = (m.ws.clone(), m.set.clone());
                let drop_member = matches!(&m.mode, Manage::Cleanup(Some(_)));
                self.run(
                    ws,
                    set,
                    repos,
                    Change::Remove { force, drop_member },
                    window,
                    cx,
                );
            }
            Manage::Archive | Manage::Resume(_) => {
                let recreate = match mode {
                    Manage::Resume(confirm) => Some(*confirm),
                    _ => None,
                };
                let (ws, set) = (ws.clone(), set.clone());
                let repos: Vec<_> = set.members.iter().map(|m| m.repo.clone()).collect();
                if recreate.is_none() && self.refuse(&set, &repos, cx) {
                    return;
                }
                self.commit(
                    ws,
                    set,
                    recreate.map_or(Commit::Archive, Commit::Resume),
                    window,
                    cx,
                );
            }
        }
    }

    fn refuse(&mut self, set: &Workset, repos: &[String], cx: &mut Context<Self>) -> bool {
        if let Some(reason) = blocker(set, repos, cx) {
            self.error = Some(format!("Refused: {reason}"));
            cx.notify();
            true
        } else {
            false
        }
    }

    fn commit(
        &mut self,
        ws: String,
        set: Workset,
        action: Commit,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.error = None;
        let core = kernel::core(cx).clone();
        let task = blocking::run(cx, {
            let ws = ws.clone();
            move || {
                match &action {
                    Commit::Archive => sessions::archive(&core, &ws, &set.id),
                    Commit::Resume(recreate) => sessions::resume(&core, &ws, &set.id, *recreate),
                    Commit::Drop(repo) => sessions::drop_member(&core, &ws, &set.id, repo),
                }
                .map(|set| (set, action))
            }
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            _ = this.update_in(cx, |s, window, cx| {
                s.busy = false;
                match result {
                    Ok((set, Commit::Resume(_))) if set.archived.is_some() => {
                        let repos = set
                            .members
                            .iter()
                            .filter(|m| m.stage == Stage::Removed)
                            .map(|m| m.repo.clone())
                            .collect();
                        s.stop = false;
                        s.result(ws.clone(), set.clone(), false, window, cx);
                        s.run(ws, set, repos, Change::Provision(None), window, cx);
                    }
                    Ok((set, action)) => {
                        let resume = matches!(action, Commit::Resume(_));
                        if resume {
                            let text = format!("Resumed {} as you left it.", set.name);
                            Activity::global(cx).update(cx, |a, cx| a.say(text, false, cx));
                        }
                        save(&ws, set, resume, cx);
                        s.close(cx);
                        if resume {
                            dock::open("project", window, cx);
                        }
                        if matches!(action, Commit::Archive) {
                            super::open_archive(window, cx);
                        }
                    }
                    Err(e) => s.fail(e, cx),
                }
            });
        })
        .detach();
        cx.notify();
    }

    fn run(
        &mut self,
        ws: String,
        mut set: Workset,
        repos: Vec<String>,
        action: Change,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.busy = true;
        self.stop = false;
        self.error = None;
        cx.spawn_in(window, async move |this, cx| {
            let mut failure = None;
            let changed = repos.clone();
            for repo in repos {
                let task = this
                    .update(cx, |s, cx| {
                        if s.stop {
                            return None;
                        }
                        let (op, task) =
                            workset::change(ws.clone(), set.id.clone(), repo, action.clone(), cx);
                        s.op = Some(op);
                        cx.notify();
                        Some(task)
                    })
                    .ok()
                    .flatten();
                let Some(task) = task else { break };
                match task.await {
                    Ok(next) => set = next,
                    Err(e) => {
                        failure = Some(e);
                        break;
                    }
                }
                _ = this.update(cx, |s, cx| {
                    s.replace(set.clone());
                    save(&ws, set.clone(), false, cx);
                    cx.notify();
                });
            }
            _ = this.update_in(cx, |s, window, cx| {
                s.op = None;
                s.busy = false;
                s.replace(set.clone());
                s.sync_retry_inputs(&set, window, cx);
                save(&ws, set.clone(), false, cx);
                if let Some(e) = failure {
                    s.fail(e, cx);
                    return;
                }
                if s.stop {
                    s.error = Some("Cancelled. No further worktrees were changed.".into());
                }
                match &s.dialog {
                    Some(Dialog::Manage(m)) => {
                        if matches!(m.mode, Manage::Cleanup(Some(_)))
                            && !set.members.iter().any(|m| m.error.is_some())
                        {
                            s.close(cx);
                        } else {
                            s.load_cleanup(cx);
                        }
                    }
                    Some(Dialog::Result(_, _, false))
                        if !s.stop
                            && set
                                .members
                                .iter()
                                .all(|m| !changed.contains(&m.repo) || m.stage == Stage::Ready) =>
                    {
                        if set.archived.is_some() {
                            s.commit(ws, set, Commit::Resume(false), window, cx);
                        } else {
                            s.close(cx);
                        }
                    }
                    _ => {}
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn result(
        &mut self,
        ws: String,
        set: Workset,
        open: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.retries.clear();
        self.sync_retry_inputs(&set, window, cx);
        self.dialog = Some(Dialog::Result(ws, set, open));
    }

    fn sync_retry_inputs(&mut self, set: &Workset, window: &mut Window, cx: &mut Context<Self>) {
        for member in set.members.iter().filter(|m| m.stage == Stage::Pending) {
            self.retries.entry(member.repo.clone()).or_insert_with(|| {
                cx.new(|cx| {
                    InputState::new(window, cx)
                        .default_value(member.branch.clone().unwrap_or_default())
                })
            });
        }
    }

    fn replace(&mut self, value: Workset) {
        match &mut self.dialog {
            Some(Dialog::Result(_, set, _)) => *set = value,
            Some(Dialog::Manage(m)) => m.set = value,
            _ => {}
        }
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            self.stop = true;
            if let Some(op) = self.op {
                Activity::cancel_op(op, cx);
            }
        } else {
            self.loading = None;
            self.dialog = None;
        }
        cx.refresh_windows();
    }

    fn fail(&mut self, error: impl ToString, cx: &mut Context<Self>) {
        self.busy = false;
        self.op = None;
        self.error = Some(error.to_string());
        cx.notify();
    }
}

enum Commit {
    Archive,
    Resume(bool),
    Drop(String),
}

fn existing<'a>(f: &'a Form, repo: &str, branch: &str) -> Option<&'a Candidate> {
    f.existing
        .iter()
        .find(|c| c.repo == repo && c.branch.as_deref() == Some(branch) && c.refusal.is_none())
}

fn valid(f: &Form, name: &str) -> bool {
    (f.set.is_some() || !name.is_empty() && !name.eq_ignore_ascii_case("Main checkouts"))
        && f.rows.iter().any(|r| r.checked)
        && f.rows
            .iter()
            .filter(|r| r.checked)
            .all(|r| r.adopt || matches!(r.preview, Some(Ok(_))))
}

fn save(ws: &str, set: Workset, select: bool, cx: &mut App) {
    Workspace::global(cx).update(cx, |w, cx| w.session_saved(ws, set, select, cx));
    kernel::invalidate::rescan(cx);
}

fn blocker(set: &Workset, repos: &[String], cx: &App) -> Option<String> {
    let paths: Vec<_> = set
        .members
        .iter()
        .filter(|m| repos.contains(&m.repo))
        .map(|m| m.path.as_str())
        .collect();
    for chat in kernel::session::all(cx) {
        let chat = chat.read(cx);
        if chat.busy() && chat.repos(cx).iter().any(|p| paths.contains(&p.as_str())) {
            return Some(format!(
                "{} is working in {}.",
                chat.spec.provider.name,
                chat.items
                    .iter()
                    .find_map(|item| match item {
                        kernel::session::Item::User(text) => text.lines().next(),
                        _ => None,
                    })
                    .unwrap_or("a chat")
            ));
        }
    }
    for path in &paths {
        for process in kernel::proc::runs(cx, path) {
            let p = process.read(cx);
            if p.running() {
                return Some(format!("{} is running.", p.name));
            }
        }
    }
    if let Some(shell) = crate::terminal::live_in(&paths, cx) {
        return Some(format!("a shell is open in {shell}."));
    }
    let activity = Activity::global(cx);
    activity
        .read(cx)
        .ops
        .iter()
        .find(|op| paths.contains(&op.repo.as_str()) || repos.contains(&op.repo))
        .map(|op| format!("{} is running.", op.label))
}

impl Render for Sheet {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(dialog) = &self.dialog else {
            return div();
        };
        let title = match dialog {
            Dialog::Form(f) => f.set.as_ref().map_or_else(
                || "New session".into(),
                |s| format!("Add a repository to {}", s.name),
            ),
            Dialog::Manage(m) => match m.mode {
                Manage::Archive => format!("Archive {}?", m.set.name),
                Manage::Resume(_) => format!("Resume {}?", m.set.name),
                Manage::Cleanup(Some(_)) => "Remove from session".into(),
                Manage::Cleanup(None) => "Remove worktrees".into(),
            },
            Dialog::Result(_, set, _) if self.busy => format!("Preparing {}", set.name),
            Dialog::Result(_, set, _) => set.name.clone(),
        };
        let content = div()
            .id("sessions.provision")
            .flex()
            .flex_col()
            .gap(GAP)
            .p(GAP * 2.)
            .max_h(window.viewport_size().height * 0.78)
            .overflow_y_scroll()
            .capture_action(cx.listener(|s, _: &Escape, _, cx| {
                s.close(cx);
                cx.stop_propagation();
            }))
            .child(
                div()
                    .text_size(text::TITLE)
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title.clone()),
            )
            .map(|d| match dialog {
                Dialog::Form(form) => self.render_form(d, form, cx),
                Dialog::Manage(m) => self.render_management(d, m, cx),
                Dialog::Result(ws, set, open) => self.render_result(d, ws, set, *open, cx),
            })
            .when_some(self.error.clone(), |d, error| {
                d.child(warning("sessions.provision.error", error, cx))
            })
            .when(self.busy && !matches!(dialog, Dialog::Result(..)), |d| {
                d.when_some(self.current_op(cx), |d, op| {
                    d.child(mono(op.line.clone(), cx))
                })
            })
            .test_support()
            .role(Role::Dialog)
            .aria_label(title);
        if self.page_mode() {
            return div()
                .size_full()
                .flex()
                .justify_center()
                .min_h_0()
                .child(content.max_w(px(760.)).w_full().max_h_full());
        }
        let handle = cx.weak_entity();
        ui::modal(
            px(560.),
            0.09,
            ui::radius::GLASS,
            move |_, cx| {
                _ = handle.update(cx, |s, cx| s.close(cx));
            },
            content,
            cx,
        )
    }
}

impl Sheet {
    fn current_op(&self, cx: &App) -> Option<kernel::activity::Op> {
        let id = self.op?;
        Activity::global(cx)
            .read(cx)
            .ops
            .iter()
            .find(|op| op.id == id)
            .cloned()
    }

    fn page_mode(&self) -> bool {
        matches!(
            &self.dialog,
            Some(Dialog::Form(Form { set: None, .. }) | Dialog::Result(_, _, true))
        )
    }

    fn render_form(
        &self,
        mut d: gpui_kit::Stateful<gpui_kit::Div>,
        f: &Form,
        cx: &mut Context<Self>,
    ) -> gpui_kit::Stateful<gpui_kit::Div> {
        let t = theme(cx);
        let adding = f.set.is_some();
        let branch = self.branch.read(cx).value();
        let n = f.rows.iter().filter(|r| r.checked).count();
        if !adding {
            d = d
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(GAP)
                        .child(button(
                            "sessions.mode.branch",
                            "A new branch",
                            Kind::Primary,
                            true,
                            |_, _| {},
                            cx,
                        ))
                        .child(button(
                            "sessions.mode.adopt",
                            "Worktrees I already have…",
                            Kind::Quiet,
                            true,
                            {
                                let handle = cx.weak_entity();
                                move |window, cx| {
                                    _ = handle.update(cx, |s, cx| s.close(cx));
                                    super::sheet::adopt(window, cx);
                                }
                            },
                            cx,
                        )),
                )
                .child(field("Name", "sessions.new.name", &self.name, cx));
        }
        d = d
            .child(field("Branch", "sessions.new.branch", &self.branch, cx))
            .when(!adding, |d| {
                d.child(detail(
                    "The same branch name in every repository. You can override it per repository.",
                    cx,
                ))
            })
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Repositories"),
            );
        for row in &f.rows {
            let i = row.i;
            let request = row.request(&branch, cx);
            let candidate = existing(f, &row.repo.path, &request.branch);
            let status = kernel::repo::Repo::get(cx, &row.repo.path).and_then(|r| r.status());
            let base_input = field("Base", ("sessions.base", i), &row.base, cx);
            let detail_text = status.and_then(|s| {
                s.branch.as_ref().map(|b| {
                    let mut text = b.clone();
                    if s.behind > 0 {
                        text.push_str(&format!(" ↓{}", s.behind));
                    }
                    if s.ahead > 0 {
                        text.push_str(&format!(" ↑{}", s.ahead));
                    }
                    if let Some(state) = &s.in_progress {
                        text.push_str(match state {
                            detached_core::types::InProgress::Merge => " · MERGING",
                            detached_core::types::InProgress::Rebase => " · REBASING",
                            detached_core::types::InProgress::CherryPick => " · CHERRY-PICKING",
                            detached_core::types::InProgress::Revert => " · REVERTING",
                            detached_core::types::InProgress::Bisect => " · BISECTING",
                        });
                    }
                    text
                })
            });
            let card = div()
                .flex()
                .flex_col()
                .gap(GAP / 2.)
                .p(GAP)
                .border_b_1()
                .border_color(t.line)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(GAP)
                        .child(
                            Checkbox::new(("sessions.repository", i))
                                .checked(row.checked)
                                .disabled(self.busy)
                                .label(row.repo.name.clone())
                                .on_click(cx.listener(move |s, checked: &bool, _, cx| {
                                    if let Some(Dialog::Form(f)) = &mut s.dialog {
                                        if f.set.is_some() {
                                            for row in &mut f.rows {
                                                row.checked = false;
                                                row.adopt = false;
                                            }
                                        }
                                        f.rows[i].checked = *checked;
                                    }
                                    s.preview(cx);
                                })),
                        )
                        .when_some(detail_text, |d, text| d.child(mono(text, cx))),
                );
            let card = if row.checked {
                card.when_some(candidate, |d, c| {
                    let label = if row.adopt {
                        "A new worktree".into()
                    } else {
                        format!("Use the worktree at {}", c.path)
                    };
                    d.child(button(
                        ("sessions.useWorktree", i),
                        label,
                        Kind::Quiet,
                        !self.busy,
                        {
                            let handle = cx.weak_entity();
                            move |_, cx| {
                                _ = handle.update(cx, |s, cx| {
                                    if let Some(Dialog::Form(f)) = &mut s.dialog {
                                        f.rows[i].adopt = !f.rows[i].adopt;
                                    }
                                    s.preview(cx);
                                });
                            }
                        },
                        cx,
                    ))
                })
                .when(!row.adopt, |d| {
                    d.child(field(
                        "Branch override",
                        ("sessions.override", i),
                        &row.branch,
                        cx,
                    ))
                    .map(|d| match &row.preview {
                        None => d
                            .child(detail("Checking refs and recipe…", cx))
                            .child(base_input),
                        Some(Err(e)) => {
                            let error = e.to_string();
                            d.child(warning(
                                ("sessions.refusal", i),
                                format!("Refused: {error}"),
                                cx,
                            ))
                            .when(error.contains("Accept the worktree recipe"), |d| {
                                let repo = row.repo.path.clone();
                                d.child(button(
                                    ("sessions.trust", i),
                                    "Trust…",
                                    Kind::Quiet,
                                    true,
                                    move |_, cx| {
                                        Workspace::global(cx).update(cx, |w, cx| {
                                            w.set_current(Some(repo.clone()), cx)
                                        });
                                        crate::settings::open(cx, "sharing");
                                    },
                                    cx,
                                ))
                            })
                            .child(base_input)
                        }
                        Some(Ok(p)) => {
                            let recipe = recipe_text(p);
                            d.child(detail(format!("{} · as of the last fetch", p.decision), cx))
                                .when(p.new_branch, |d| {
                                    let base = row.base.clone();
                                    let options = p.bases.clone();
                                    let caption = request.base.as_ref().map_or_else(
                                        || {
                                            format!(
                                                "{} · recipe",
                                                p.decision.trim_start_matches("New branch from ")
                                            )
                                        },
                                        |b| format!("{b} · chosen"),
                                    );
                                    d.child(button(
                                        ("sessions.basePicker", i),
                                        caption,
                                        Kind::Quiet,
                                        !self.busy,
                                        move |window, cx| {
                                            let mut menu = ui::Menu::default();
                                            for (label, value) in [
                                                ("Recipe base".to_string(), String::new()),
                                                ("The main checkout’s HEAD".into(), "HEAD".into()),
                                            ]
                                            .into_iter()
                                            .chain(options.iter().map(|b| (b.clone(), b.clone())))
                                            {
                                                let base = base.clone();
                                                menu = menu.item(label, move |window, cx| {
                                                    base.update(cx, |input, cx| {
                                                        input.set_value(&value, window, cx)
                                                    });
                                                });
                                            }
                                            menu.show(window.mouse_position(), window, cx);
                                        },
                                        cx,
                                    ))
                                    .child(base_input)
                                })
                                .child(mono(p.path.display().to_string(), cx))
                                .child(detail(recipe, cx))
                        }
                    })
                })
                .when(status.is_some_and(|s| s.in_progress.is_some()), |d| {
                    d.child(detail(
                        "The main checkout has an operation in progress. A worktree is unaffected.",
                        cx,
                    ))
                })
            } else {
                card.child(detail("Not in the session", cx))
            };
            d = d.child(card);
        }
        if f.rows.is_empty() {
            d = d.child(detail("No available repositories in this workspace.", cx));
        }
        let label = if adding {
            f.rows.iter().find(|r| r.checked).map_or_else(
                || "Add repository".into(),
                |r| format!("Add {}", r.repo.name),
            )
        } else {
            match n {
                0 => "Create worktrees".into(),
                1 => "Create 1 worktree".into(),
                n => format!("Create {n} worktrees"),
            }
        };
        d.child(detail(if adding {
            "Chats on the entire session can write there from their next message."
        } else {
            "Previews use the refs at hand. Create fetches first, so a branch that has appeared on origin is tracked."
        }, cx))
            .child(self.footer(button("sessions.create.submit", label, Kind::Primary,
                !self.busy && valid(f, &self.name.read(cx).value()),
                self.command(Self::submit, cx), cx), cx))
    }

    fn render_management(
        &self,
        mut d: gpui_kit::Stateful<gpui_kit::Div>,
        m: &Management,
        cx: &mut Context<Self>,
    ) -> gpui_kit::Stateful<gpui_kit::Div> {
        let t = theme(cx);
        let run = self.command(Self::commit_manage, cx);
        match &m.mode {
            Manage::Archive => {
                return d
                    .child(format!(
                        "Hides {}. Keeps its worktrees, changes, branches, chats and tabs.",
                        m.set.name
                    ))
                    .child(self.footer(
                        button(
                            "sessions.archive.confirm",
                            "Archive",
                            Kind::Primary,
                            !self.busy,
                            run,
                            cx,
                        ),
                        cx,
                    ));
            }
            Manage::Resume(_) => {
                for member in &m.set.members {
                    if !member.state.usable() && member.stage != Stage::Pending {
                        d = d.child(detail(
                            format!(
                                "Recreate {} from {}.",
                                member.path,
                                member.branch.as_deref().unwrap_or("its branch")
                            ),
                            cx,
                        ));
                    }
                }
                return d.child(self.footer(
                    button(
                        "sessions.resume.confirm",
                        "Recreate and resume",
                        Kind::Primary,
                        !self.busy,
                        run,
                        cx,
                    ),
                    cx,
                ));
            }
            Manage::Cleanup(scope) => {
                if let Some(repo) = &m.confirm {
                    let row = m.rows.iter().find(|(member, _)| &member.repo == repo);
                    if let Some((member, preview)) = row {
                        return d
                            .child(warning(
                                "sessions.remove.discard",
                                format!(
                                    "Discards {} in {}. Uncommitted files cannot be recovered.",
                                    preview
                                        .as_ref()
                                        .map_or_else(|_| "worktree contents".into(), cleanup_text),
                                    member.path
                                ),
                                cx,
                            ))
                            .child(self.footer(
                                button(
                                    "sessions.remove.discard.confirm",
                                    "Discard and remove",
                                    Kind::Danger,
                                    !self.busy,
                                    run,
                                    cx,
                                ),
                                cx,
                            ));
                    }
                }
                if self.loading.is_some() {
                    d = d.child(ui::spinner(t.accent));
                }
                let mut clean = 0;
                for (i, (member, preview)) in m
                    .rows
                    .iter()
                    .enumerate()
                    .filter(|(_, (m, _))| scope.as_ref().is_none_or(|r| r == &m.repo))
                {
                    let repo = member.repo.clone();
                    let last = m.set.members.iter().find(|m| m.repo == repo);
                    let error = last.and_then(|m| m.error.as_ref());
                    let ready = preview.as_ref().is_ok_and(Cleanup::clean);
                    if ready {
                        clean += 1;
                    }
                    d = d.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(GAP / 2.)
                            .border_b_1()
                            .border_color(t.line)
                            .py(GAP)
                            .child(mono(member.path.clone(), cx))
                            .map(|d| match preview {
                                Ok(p) => d
                                    .child(detail(cleanup_text(p), cx))
                                    .child(detail(
                                        match &p.teardown {
                                            Ok(Some(script)) => format!("Teardown: {script}"),
                                            Ok(None) => "No teardown".into(),
                                            Err(e) => e.to_string(),
                                        },
                                        cx,
                                    ))
                                    .child(detail(
                                        if ready {
                                            "Remove the worktree and keep its branch"
                                        } else {
                                            "Keep the worktree"
                                        },
                                        cx,
                                    ))
                                    .when(p.locked, |d| {
                                        d.child(warning(
                                            ("sessions.locked", i),
                                            "Locked. git worktree unlock frees it.",
                                            cx,
                                        ))
                                    })
                                    .when(!p.locked && (!ready || error.is_some()), |d| {
                                        let handle = cx.weak_entity();
                                        let repo = repo.clone();
                                        d.child(button(
                                            ("sessions.force", i),
                                            "Remove anyway…",
                                            Kind::Danger,
                                            !self.busy,
                                            move |_, cx| {
                                                _ = handle.update(cx, |s, cx| {
                                                    if let Some(Dialog::Manage(m)) = &mut s.dialog {
                                                        m.confirm = Some(repo.clone());
                                                    }
                                                    cx.notify();
                                                });
                                            },
                                            cx,
                                        ))
                                    }),
                                Err(e) => d.child(warning(
                                    ("sessions.cleanup.error", i),
                                    e.to_string(),
                                    cx,
                                )),
                            })
                            .when_some(error, |d, e| {
                                d.child(warning(("sessions.cleanup.result", i), e.clone(), cx))
                            })
                            .when(scope.is_some(), |d| {
                                let handle = cx.weak_entity();
                                let (ws, set) = (m.ws.clone(), m.set.clone());
                                d.child(button(
                                    ("sessions.drop", i),
                                    "Keep the worktree, drop it from the session",
                                    Kind::Quiet,
                                    !self.busy,
                                    move |window, cx| {
                                        _ = handle.update(cx, |s, cx| {
                                            if !s.refuse(&set, std::slice::from_ref(&repo), cx) {
                                                s.commit(
                                                    ws.clone(),
                                                    set.clone(),
                                                    Commit::Drop(repo.clone()),
                                                    window,
                                                    cx,
                                                );
                                            }
                                        });
                                    },
                                    cx,
                                ))
                            }),
                    );
                }
                for member in m.set.members.iter().filter(|m| m.stage == Stage::Removed) {
                    d = d.child(detail(format!("Removed {}", member.path), cx));
                }
                let label = match clean {
                    0 => "Remove clean worktrees".into(),
                    1 => "Remove 1 clean worktree".into(),
                    n => format!("Remove {n} clean worktrees"),
                };
                d.child(detail(
                    "A kept branch does not bring back an uncommitted file.",
                    cx,
                ))
                .when(clean == 0 && self.loading.is_none(), |d| {
                    d.child(detail("None is clean.", cx))
                })
                .child(self.footer(
                    button(
                        "sessions.remove.clean",
                        label,
                        Kind::Danger,
                        clean > 0 && !self.busy,
                        run,
                        cx,
                    ),
                    cx,
                ))
            }
        }
    }

    fn render_result(
        &self,
        mut d: gpui_kit::Stateful<gpui_kit::Div>,
        ws: &str,
        set: &Workset,
        open: bool,
        cx: &mut Context<Self>,
    ) -> gpui_kit::Stateful<gpui_kit::Div> {
        let count = set.members.iter().filter(|m| m.state.usable()).count();
        if !self.busy {
            let message = if count == set.members.len() {
                format!("{} is ready.", set.name)
            } else {
                format!(
                    "{} of {} repositories have worktrees.",
                    count,
                    set.members.len()
                )
            };
            d = d.child(detail(message, cx));
        }
        for (i, m) in set.members.iter().enumerate() {
            let repo = m.repo.clone();
            let (workspace, workset) = (ws.to_owned(), set.clone());
            let path = m.path.clone();
            let handle = cx.weak_entity();
            d = d.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(GAP / 2.)
                    .py(GAP)
                    .child(mono(path.clone(), cx))
                    .child(detail(m.state.label(), cx))
                    .children(m.steps.iter().map(|step| {
                        detail(step.clone(), cx).when(step.starts_with("Fetch failed"), |d| {
                            d.text_color(theme(cx).warning)
                        })
                    }))
                    .when_some(
                        self.current_op(cx).filter(|op| op.repo == m.repo),
                        |d, op| {
                            d.child(
                                div()
                                    .id("sessions.provision.progress")
                                    .test_support()
                                    .aria_label(op.line)
                                    .children(op.recent.into_iter().map(|line| mono(line, cx))),
                            )
                        },
                    )
                    .when_some(m.error.clone(), |d, e| {
                        let (message, tail) = e.split_once('\n').unwrap_or((&e, ""));
                        d.child(warning(
                            ("sessions.result.error", i),
                            message.to_owned(),
                            cx,
                        ))
                        .when(!tail.trim().is_empty(), |d| {
                            d.child(
                                mono(tail.to_owned(), cx)
                                    .rounded(ui::radius::WIDGET)
                                    .bg(theme(cx).code_block_bg)
                                    .p(GAP),
                            )
                        })
                    })
                    .when(
                        !self.busy && !m.state.usable() || !self.busy && m.stage != Stage::Ready,
                        |d| {
                            let ws = workspace.clone();
                            let set = workset.clone();
                            let repo = repo.clone();
                            let handle = handle.clone();
                            let input = self
                                .retries
                                .get(&m.repo)
                                .filter(|_| m.stage == Stage::Pending)
                                .cloned();
                            let label = match m.stage {
                                Stage::Install => "Retry install",
                                Stage::Copy | Stage::Adding => "Retry copy",
                                Stage::Pending => "Create",
                                _ => "Create it again",
                            };
                            d.when_some(input.as_ref(), |d, input| {
                                d.child(field(
                                    "Another branch name",
                                    ("sessions.retry.branch", i),
                                    input,
                                    cx,
                                ))
                            })
                            .child(button(
                                ("sessions.retry", i),
                                label,
                                Kind::Quiet,
                                true,
                                move |window, cx| {
                                    _ = handle.update(cx, |s, cx| {
                                        let branch = input
                                            .as_ref()
                                            .map(|input| input.read(cx).value().to_string());
                                        s.run(
                                            ws.clone(),
                                            set.clone(),
                                            vec![repo.clone()],
                                            Change::Provision(branch),
                                            window,
                                            cx,
                                        );
                                    });
                                },
                                cx,
                            ))
                        },
                    )
                    .when(
                        !self.busy && m.state.usable() && m.stage != Stage::Ready,
                        |d| {
                            d.child(button(
                                ("sessions.term", i),
                                "Terminal there",
                                Kind::Quiet,
                                true,
                                {
                                    let (path, ws, set) =
                                        (path.clone(), workspace.clone(), workset.clone());
                                    let handle = handle.clone();
                                    move |window, cx| {
                                        save(&ws, set.clone(), true, cx);
                                        _ = handle.update(cx, |s, cx| s.close(cx));
                                        dock::show(
                                            Doc::Terminal {
                                                id: 0,
                                                cwd: path.clone(),
                                            },
                                            window,
                                            cx,
                                        )
                                    }
                                },
                                cx,
                            ))
                            .child(button(
                                ("sessions.removeFailed", i),
                                "Remove this worktree",
                                Kind::Danger,
                                true,
                                move |_, cx| {
                                    manage(
                                        workset.clone(),
                                        Manage::Cleanup(Some(repo.clone())),
                                        cx,
                                    );
                                },
                                cx,
                            ))
                        },
                    ),
            );
        }
        let enabled = !self.busy && open && count > 0;
        let (set, ws) = (set.clone(), ws.to_owned());
        let title = format!("Open {}", set.name);
        d.child(self.footer(
            button(
                "sessions.openCreated",
                title,
                Kind::Primary,
                enabled,
                {
                    let handle = cx.weak_entity();
                    move |window, cx| {
                        save(&ws, set.clone(), true, cx);
                        _ = handle.update(cx, |s, cx| s.close(cx));
                        dock::open("project", window, cx);
                    }
                },
                cx,
            ),
            cx,
        ))
    }

    fn footer(&self, button: AnyElement, cx: &Context<Self>) -> gpui_kit::Div {
        div()
            .flex()
            .justify_end()
            .gap(GAP)
            .child(self.action(
                "sessions.provision.cancel",
                "Cancel",
                Kind::Quiet,
                true,
                |s, _, cx| s.close(cx),
                cx,
            ))
            .child(button)
    }

    fn command(
        &self,
        action: fn(&mut Self, &mut Window, &mut Context<Self>),
        cx: &Context<Self>,
    ) -> impl Fn(&mut Window, &mut App) + 'static {
        let handle = cx.weak_entity();
        move |window, cx| {
            _ = handle.update(cx, |s, cx| action(s, window, cx));
        }
    }

    fn action(
        &self,
        id: &'static str,
        label: &'static str,
        kind: Kind,
        enabled: bool,
        action: fn(&mut Self, &mut Window, &mut Context<Self>),
        cx: &Context<Self>,
    ) -> AnyElement {
        button(id, label, kind, enabled, self.command(action, cx), cx)
    }
}

fn field(
    label: &str,
    id: impl Into<ElementId>,
    input: &Entity<InputState>,
    cx: &App,
) -> gpui_kit::Div {
    div()
        .flex()
        .flex_col()
        .gap(GAP / 3.)
        .child(detail(label.to_owned(), cx))
        .child(ui::input(input, cx).id(id))
}

fn detail(text: impl Into<gpui_kit::SharedString>, cx: &App) -> gpui_kit::Div {
    div()
        .text_color(theme(cx).muted)
        .text_size(text::SMALL)
        .child(text.into())
}

fn mono(text: impl Into<gpui_kit::SharedString>, cx: &App) -> gpui_kit::Div {
    detail(text, cx).font_family(ui::theme::mono(cx))
}

fn warning(
    id: impl Into<ElementId>,
    message: impl Into<gpui_kit::SharedString>,
    cx: &App,
) -> AnyElement {
    let message = message.into();
    div()
        .id(id)
        .test_support()
        .aria_label(message.clone())
        .text_color(theme(cx).warning)
        .child(message)
        .into_any_element()
}

fn recipe_text(p: &Preview) -> String {
    let mut steps = vec![];
    if !p.recipe.copies.is_empty() {
        steps.push(format!("copies {}", p.recipe.copies.join(", ")));
    }
    if let Some(install) = &p.recipe.install {
        steps.push(format!("runs {install}"));
    }
    if steps.is_empty() {
        "No recipe".into()
    } else {
        format!("Recipe: {}", steps.join(" · "))
    }
}

pub(super) fn cleanup_text(p: &Cleanup) -> String {
    let mut parts = vec![];
    if p.changes > 0 {
        parts.push(format!("{} changed file(s)", p.changes));
    }
    if p.untracked > 0 {
        parts.push(format!("{} untracked file(s)", p.untracked));
    }
    if p.submodules {
        parts.push("has submodules".into());
    }
    if parts.is_empty() {
        parts.push("clean".into());
    }
    if !p.ignored.is_empty() {
        parts.push(format!("ignored: {}", p.ignored.join(", ")));
    }
    parts.join(" · ")
}

enum Kind {
    Primary,
    Quiet,
    Danger,
}

fn button(
    id: impl Into<ElementId>,
    label: impl Into<gpui_kit::SharedString>,
    kind: Kind,
    enabled: bool,
    click: impl Fn(&mut Window, &mut App) + 'static,
    cx: &App,
) -> AnyElement {
    let label = label.into();
    match kind {
        Kind::Primary => ui::primary_button(id, label, cx),
        Kind::Quiet => ui::quiet_button(id, label, cx),
        Kind::Danger => ui::danger_button(id, label, cx),
    }
    .test_support()
    .when(!enabled, |d| d.opacity(0.5))
    .on_click(move |_, window, cx| {
        if enabled {
            click(window, cx);
        }
    })
    .into_any_element()
}
