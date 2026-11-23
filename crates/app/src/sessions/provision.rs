use std::collections::HashMap;

use detached_core::git::worktree::{Cleanup, Preview};
use detached_core::sessions::{self, Candidate, Member, Request, Stage, Workset};
use detached_core::types::RepoInfo;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::{
    AnyView, App, AppContext, Context, Entity, Global, IntoElement, Render, Task, WeakEntity,
    Window,
};
use kernel::activity::Activity;
use kernel::blocking;
use kernel::workset::{self, Change};
use kernel::workspace::Workspace;

use crate::dock;

mod view;
pub(super) use view::cleanup_text;

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
            previous: None,
        }
    });
    cx.set_global(Controller(sheet.downgrade()));
    cx.new(|_| Overlay(sheet)).into()
}

struct Overlay(Entity<Sheet>);

impl Render for Overlay {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.0.read(cx).page_mode() {
            gpui_kit::Empty.into_any_element()
        } else {
            self.0.clone().into_any_element()
        }
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
                failures: HashMap::new(),
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
    failures: HashMap<String, String>,
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
    previous: Option<Form>,
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
                    if !adding && let Some(Dialog::Form(form)) = s.dialog.take() {
                        s.previous = Some(form);
                    }
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
        self.stop = false;
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
                let drop_member = scope.is_some() && set.archived.is_none();
                if repos.is_empty() || self.refuse(&set, &repos, cx) {
                    return;
                }
                let Some(Dialog::Manage(m)) = &mut self.dialog else {
                    return;
                };
                m.confirm = None;
                let (ws, set) = (m.ws.clone(), m.set.clone());
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
                    Err(e) => {
                        if let Some(Dialog::Manage(m)) = &mut s.dialog
                            && matches!(m.mode, Manage::Resume(false))
                        {
                            m.mode = Manage::Resume(true);
                        }
                        s.fail(e, cx);
                    }
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
            let discarded = if set.draft {
                let (ws, id) = (ws.clone(), set.id.clone());
                let task = this.update(cx, |_, cx| {
                    let core = kernel::core(cx).clone();
                    blocking::run(cx, move || sessions::discard_draft(&core, &ws, &id))
                });
                match task {
                    Ok(task) => task.await.unwrap_or(false),
                    Err(_) => false,
                }
            } else {
                false
            };
            _ = this.update_in(cx, |s, window, cx| {
                s.op = None;
                s.busy = false;
                if discarded && let Some(mut form) = s.previous.take() {
                    form.failures = set
                        .members
                        .into_iter()
                        .filter_map(|m| m.error.map(|error| (m.repo, error)))
                        .collect();
                    s.dialog = Some(Dialog::Form(form));
                    if s.stop {
                        s.error = Some("Cancelled before creating a worktree.".into());
                    } else if let Some(e) = failure {
                        s.fail(e, cx);
                    }
                    s.preview(cx);
                    return;
                }
                s.previous = None;
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
            self.previous = None;
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
    if !set.draft {
        Workspace::global(cx).update(cx, |w, cx| w.session_saved(ws, set, select, cx));
    }
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
