//! The Processes app: the current repo's defined processes, started, stopped and restarted by hand,
//! the selected one's log and what agents may do with it, and a form that defines one. A shell is
//! not a process: agents never read or type into one.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use detached_core::error::Result;
use detached_core::processes::{self, Agents, Definition, Entry};
use detached_core::secrets;
use gpui_kit::base::TestSupportExt;
use gpui_kit::base::dock::DockPlacement;
use gpui_kit::component::IndexPath;
use gpui_kit::component::input::InputState;
use gpui_kit::component::select::SelectState;
use gpui_kit::{
    AnyElement, AnyView, App, AppContext, Context, Entity, EntityId, FontWeight, Global,
    InteractiveElement, IntoElement, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, Subscription, Task, Window, div, prelude::FluentBuilder,
    px,
};
use kernel::blocking;
use kernel::proc::{self as procs, Proc, State};
use kernel::resource::Interest;
use kernel::settings::Settings;
use kernel::workspace::Workspace;

use crate::actions::{self, Group, Registration};
use crate::dock::{self, AppSpec, DocKind};
use crate::ui::page::{Opt, Pick, button, labeled, muted, row, select};
use crate::ui::{self, Edge, text, theme};
use kernel::doc::Doc;

/// The lines of the log drawn.
const SHOWN: usize = 500;

pub fn register(cx: &mut App) -> Vec<Registration> {
    dock::register_app(
        cx,
        AppSpec {
            name: "processes",
            id: "bottom.Processes",
            title: "Processes",
            action: Some("panel.processes"),
            home: (DockPlacement::Bottom, 280.),
            view: |w, cx| cx.new(|cx| Processes::new(w, None, cx)).into(),
            shown: None,
        },
    );
    dock::register_doc(cx, "process", ProcessDoc);
    vec![actions::register(
        cx,
        "panel.processes",
        "Processes",
        Group::View,
        |w, cx| {
            if let Some(repo) = dock::bound_repo(cx) {
                dock::open_doc(Doc::Process { repo }, true, w, cx);
            }
        },
    )]
}

struct ProcessDoc;

#[derive(Default)]
struct RevealRun(Option<(String, String)>);

impl Global for RevealRun {}

pub(crate) fn show_run(repo: String, name: String, w: &mut Window, cx: &mut App) {
    cx.default_global::<RevealRun>().0 = Some((repo.clone(), name));
    dock::open_doc(Doc::Process { repo }, true, w, cx);
}

impl DocKind for ProcessDoc {
    fn category(&self, _: &Doc) -> &'static str {
        "code"
    }

    fn view(&self, doc: &mut Doc, _: bool, w: &mut Window, cx: &mut App) -> AnyView {
        let repo = doc.repo().map(String::from);
        cx.new(|cx| Processes::new(w, repo, cx)).into()
    }

    fn id(&self, _: &Doc) -> SharedString {
        "doc.process".into()
    }

    fn title(&self, doc: &Doc, _: Option<&AnyView>, cx: &App) -> SharedString {
        format!(
            "Processes · {}",
            Workspace::get(cx).name_of(doc.repo().unwrap_or_default())
        )
        .into()
    }

    fn running(&self, doc: &Doc, _: Option<&AnyView>, cx: &App) -> Option<String> {
        let names = running(doc.repo()?, cx);
        (!names.is_empty()).then(|| format!("Running: {}", names.join(", ")))
    }

    fn activated(&self, view: &AnyView, _: &mut Window, cx: &mut App) {
        if let Ok(view) = view.clone().downcast::<Processes>()
            && let Some((repo, name)) = cx.try_global::<RevealRun>().and_then(|r| r.0.clone())
            && view.read(cx).fixed.as_ref() == Some(&repo)
        {
            cx.default_global::<RevealRun>().0 = None;
            view.update(cx, |v, cx| {
                (v.selected, v.form) = (Some(name), None);
                cx.notify();
            });
        }
    }

    fn closing(&self, doc: &Doc, cx: &mut App) {
        if let Some(repo) = doc.repo() {
            for name in running(repo, cx) {
                procs::stop(cx, repo, &name, false).detach();
            }
        }
    }
}

fn running(repo: &str, cx: &App) -> Vec<String> {
    procs::runs(cx, repo)
        .filter_map(|p| {
            let p = p.read(cx);
            p.running().then(|| p.name.clone())
        })
        .collect()
}

struct Form {
    /// The definition it changes; None defines a new one.
    editing: Option<Definition>,
    name: Entity<InputState>,
    command: Entity<InputState>,
    folder: Entity<InputState>,
    ready: Entity<InputState>,
    /// "" is no profile.
    profile: Pick,
}

struct Processes {
    fixed: Option<String>,
    repo: Option<String>,
    defs: Option<Interest<Vec<Entry>>>,
    selected: Option<String>,
    form: Option<Form>,
    error: Option<String>,
    busy: bool,
    runs: HashMap<EntityId, Subscription>,
    _workspace: Subscription,
    _settings: Subscription,
}

impl Processes {
    fn new(_: &mut Window, fixed: Option<String>, cx: &mut Context<Self>) -> Self {
        let mut this = Processes {
            fixed,
            repo: None,
            defs: None,
            selected: None,
            form: None,
            error: None,
            busy: false,
            runs: HashMap::new(),
            _workspace: Workspace::observe(cx, |this: &mut Self, cx| this.follow(cx)),
            _settings: cx.observe(&Settings::global(cx), |_, _, cx| cx.notify()),
        };
        this.follow(cx);
        this
    }

    fn follow(&mut self, cx: &mut Context<Self>) {
        let repo = self
            .fixed
            .clone()
            .or_else(|| Workspace::get(cx).current().map(String::from));
        if self.repo != repo {
            self.defs = (repo.as_ref()).map(|r| Interest::view(procs::definitions(cx, r), cx));
            (self.repo, self.selected, self.form, self.error) = (repo, None, None, None);
        }
        cx.notify();
    }

    /// The definitions, then each run whose definition is gone, which keeps its Stop.
    fn entries(&self, cx: &App) -> Vec<Row> {
        let defs = self.defs.as_ref().and_then(|d| d.read(cx).value.as_ref());
        let defs = defs.into_iter().flatten().map(|e: &Entry| Row {
            name: e.name.clone(),
            def: e.def.clone(),
        });
        let mut rows: Vec<Row> = defs.collect();
        let repo = self.repo.as_deref().unwrap_or_default();
        for p in procs::runs(cx, repo) {
            let p = p.read(cx);
            if p.running() && !rows.iter().any(|r| r.name == p.name) {
                let def = Err("Its definition is gone.".to_string());
                rows.push(Row {
                    name: p.name.clone(),
                    def,
                });
            }
        }
        rows
    }

    /// Shows what `task` failed with, or closes the form it started from, and the definitions as
    /// they are after it.
    fn after(&mut self, task: Task<Result<()>>, cx: &mut Context<Self>) {
        self.busy = true;
        self.error = None;
        let form = self.form.as_ref().map(|f| f.name.entity_id());
        cx.spawn(async move |this, cx| {
            let done = task.await;
            this.update(cx, |this, cx| {
                this.busy = false;
                match done {
                    Ok(()) if this.form.as_ref().map(|f| f.name.entity_id()) == form => {
                        this.form = None
                    }
                    Ok(()) => {}
                    Err(e) => this.error = Some(e.to_string()),
                }
                if let Some(d) = &this.defs {
                    d.refresh(cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// Saves `def` and selects it.
    fn save(&mut self, def: Definition, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.clone() else {
            return;
        };
        let core = kernel::core(cx).clone();
        self.selected = Some(def.name.clone());
        let saved = blocking::run(cx, move || processes::save(&core, Path::new(&repo), &def));
        self.after(saved, cx);
    }

    fn op(&mut self, name: &str, op: Op, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.clone() else {
            return;
        };
        let task = match op {
            Op::Start => procs::start(cx, &repo, name, false),
            Op::Stop => procs::stop(cx, &repo, name, false),
            Op::Restart => procs::restart(cx, &repo, name, false),
            Op::Delete => {
                let (core, name) = (kernel::core(cx).clone(), name.to_string());
                let stopped = procs::stop(cx, &repo, &name, false);
                let remove = move || processes::remove(&core, Path::new(&repo), &name);
                cx.spawn(async move |_, cx| {
                    stopped.await?;
                    cx.update(|cx| blocking::run(cx, remove)).await
                })
            }
        };
        self.after(task, cx);
    }

    fn edit(&mut self, def: Option<Definition>, window: &mut Window, cx: &mut Context<Self>) {
        let d = def.clone().unwrap_or_default();
        let mut field = |value: &str, hint: &'static str, cx: &mut Context<Self>| {
            let value = value.to_string();
            cx.new(|cx| (InputState::new(window, cx).placeholder(hint)).default_value(value))
        };
        let mut opts = vec![Opt(String::new(), "No env profile".into())];
        let profiles: BTreeMap<String, Vec<String>> =
            Settings::get(cx).block(secrets::BLOCK).unwrap_or_default();
        opts.extend(profiles.into_keys().map(|p| Opt(p.clone(), p.into())));
        let current = d.profile.clone().unwrap_or_default();
        let ix = opts.iter().position(|o| o.0 == current).map(IndexPath::new);
        self.form = Some(Form {
            editing: def,
            name: field(&d.name, "Name", cx),
            command: field(&d.command, "Command, such as npm run dev", cx),
            folder: field(&d.folder, "Folder in the repo, empty for its root", cx),
            ready: field(
                d.ready.as_deref().unwrap_or_default(),
                "Ready when a line matches, such as Local:.*http",
                cx,
            ),
            profile: cx.new(|cx| SelectState::new(opts, ix, window, cx)),
        });
        self.error = None;
        cx.notify();
    }

    fn submit(&mut self, cx: &mut Context<Self>) {
        let Some(f) = &self.form else { return };
        let text = |i: &Entity<InputState>| i.read(cx).value().trim().to_string();
        let some = |s: String| Some(s).filter(|s| !s.is_empty());
        let old = f.editing.clone();
        let def = Definition {
            name: old
                .as_ref()
                .map_or_else(|| text(&f.name), |d| d.name.clone()),
            command: text(&f.command),
            folder: text(&f.folder),
            profile: f.profile.read(cx).selected_value().cloned().and_then(some),
            ready: some(text(&f.ready)),
            agents: old.map_or(Agents::Off, |d| d.agents),
        };
        self.save(def, cx);
    }

    fn list(&self, entries: &[Row], cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let rows = entries.iter().map(|e| {
            let run = (self.repo.as_ref()).and_then(|r| procs::get(cx, r, &e.name));
            let (color, said) = match (&e.def, &run) {
                (Err(why), _) => (theme(cx).warning, why.clone()),
                (Ok(_), Some(p)) => summary(p.read(cx), cx),
                (Ok(_), None) => (theme(cx).muted, "Stopped".into()),
            };
            let on = self.selected.as_ref() == Some(&e.name);
            let name = e.name.clone();
            ui::row(
                SharedString::from(format!("processes.row.{}", e.name)),
                on,
                cx,
            )
            .test_support()
            .aria_label(SharedString::from(format!("{} · {said}", e.name)))
            .h(px(26.))
            .child(ui::dot(color))
            .child(div().flex_1().min_w_0().truncate().child(e.name.clone()))
            .child(
                div()
                    .flex_none()
                    .max_w(px(110.))
                    .truncate()
                    .text_size(text::SMALL)
                    .text_color(theme(cx).muted)
                    .child(said),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                (this.selected, this.form) = (Some(name.clone()), None);
                cx.notify();
            }))
        });
        let list = div()
            .id("processes.list")
            .size_full()
            .flex()
            .flex_col()
            .p(px(4.))
            .overflow_y_scroll()
            .children(rows);
        ui::pane("processes.list", Edge::Right, 240., 160., 400.)
            .border_r_1()
            .border_color(theme(cx).line)
            .child(list)
    }

    fn detail(&self, e: &Row, cx: &mut Context<Self>) -> AnyElement {
        let repo = self.repo.clone().unwrap_or_default();
        let def = match &e.def {
            Ok(def) => def.clone(),
            // A run outlives its definition's change: it keeps Stop and its log.
            Err(why) => {
                let run = procs::get(cx, &repo, &e.name).map(|p| p.read(cx));
                let running = run.is_some_and(Proc::running);
                let lines = run.map_or(vec![], shown);
                let name = e.name.clone();
                let said = format!("{}: {why} Accept or fix it on the Sharing page.", e.name);
                return div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .p(px(8.))
                    .gap(px(6.))
                    .child(labeled("processes.untrusted", said))
                    .child(
                        row()
                            .when(running, |r| {
                                r.child(button("processes.stop", "Stop", !self.busy, cx, {
                                    move |this, _, cx| this.op(&name, Op::Stop, cx)
                                }))
                            })
                            .child(button("processes.sharing", "Open Sharing", true, cx, {
                                |_, _, cx| crate::settings::open(cx, "sharing")
                            })),
                    )
                    .child(log(lines, cx))
                    .into_any_element();
            }
        };
        let run = procs::get(cx, &repo, &def.name).map(|p| p.read(cx));
        let running = run.is_some_and(Proc::running);
        let said = run.map_or("Stopped".into(), |p| summary(p, cx).1);
        let lines = run.map_or(vec![], shown);
        let name = def.name.clone();
        let opn = |op: Op| {
            let name = name.clone();
            move |this: &mut Self, _: &mut Window, cx: &mut Context<Self>| this.op(&name, op, cx)
        };
        let edit = def.clone();
        let buttons = row()
            .when(!running, |r| {
                r.child(button(
                    "processes.start",
                    "Start",
                    !self.busy,
                    cx,
                    opn(Op::Start),
                ))
            })
            .when(running, |r| {
                r.child(button(
                    "processes.stop",
                    "Stop",
                    !self.busy,
                    cx,
                    opn(Op::Stop),
                ))
                .child(button(
                    "processes.restart",
                    "Restart",
                    !self.busy,
                    cx,
                    opn(Op::Restart),
                ))
            })
            .child(button(
                "processes.edit",
                "Edit",
                true,
                cx,
                move |this, w, cx| this.edit(Some(edit.clone()), w, cx),
            ))
            .child(button(
                "processes.delete",
                "Delete",
                !self.busy,
                cx,
                opn(Op::Delete),
            ));
        let (level, command) = (def.agents, def.command.clone());
        let this = cx.weak_entity();
        let agents = ui::segmented(
            "processes.agents",
            &[
                (Agents::Off, "Agents: off"),
                (Agents::Read, "Read its log"),
                (Agents::Start, "Start, stop and restart"),
            ],
            level,
            move |agents, _, cx| {
                let def = Definition {
                    agents,
                    ..def.clone()
                };
                this.update(cx, |this, cx| this.save(def, cx)).ok();
            },
            cx,
        );
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .p(px(8.))
            .gap(px(6.))
            .child(
                row()
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(e.name.clone()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .font_family(theme::mono(cx))
                            .text_size(text::SMALL)
                            .text_color(theme(cx).muted)
                            .child(command),
                    )
                    .child(labeled("processes.state", said)),
            )
            .child(buttons)
            .child(agents)
            .when(level == Agents::Start, |d| {
                d.child(
                    labeled("processes.warning", warning(e, cx))
                        .text_size(text::SMALL)
                        .text_color(theme(cx).warning),
                )
            })
            .child(log(lines, cx))
            .into_any_element()
    }

    fn editor(&self, f: &Form, cx: &mut Context<Self>) -> AnyElement {
        let field = |state: &Entity<InputState>, id: &'static str| {
            div().flex_1().min_w_0().child(ui::input(state, cx).id(id))
        };
        let name = match &f.editing {
            Some(d) => div().child(labeled("processes.editing", d.name.clone())),
            None => div()
                .w(px(160.))
                .child(ui::input(&f.name, cx).id("processes.name")),
        };
        div()
            .id("processes.form")
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .p(px(8.))
            .overflow_y_scroll()
            .child(
                row()
                    .child(name)
                    .child(field(&f.command, "processes.command")),
            )
            .child(
                row()
                    .child(field(&f.folder, "processes.folder"))
                    .child(field(&f.ready, "processes.ready"))
                    .child(
                        div()
                            .w(px(180.))
                            .child(select(&f.profile, "processes.profile", cx)),
                    ),
            )
            .child(
                row()
                    .child(button(
                        "processes.save",
                        "Save",
                        !self.busy,
                        cx,
                        |this, _, cx| this.submit(cx),
                    ))
                    .child(button(
                        "processes.cancel",
                        "Cancel",
                        true,
                        cx,
                        |this, _, cx| {
                            (this.form, this.error) = (None, None);
                            cx.notify();
                        },
                    )),
            )
            .into_any_element()
    }
}

#[derive(Clone, Copy)]
enum Op {
    Start,
    Stop,
    Restart,
    Delete,
}

/// A definition, or a run whose definition no longer loads.
struct Row {
    name: String,
    def: std::result::Result<Definition, String>,
}

/// The end of a run's log that is drawn.
fn shown(p: &Proc) -> Vec<String> {
    let skip = p.log.len().saturating_sub(SHOWN);
    p.log.iter().skip(skip).cloned().collect()
}

fn log(lines: Vec<String>, cx: &App) -> impl IntoElement {
    div()
        .id("processes.log")
        .test_support()
        .aria_label(SharedString::from(lines.join("\n")))
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .p(px(6.))
        .rounded(ui::radius::ITEM)
        .bg(theme(cx).code)
        .font_family(theme::mono(cx))
        .text_size(text::SMALL)
        .children(lines.into_iter().map(|l| div().child(l)))
}

/// A run's color and state in words.
fn summary(p: &Proc, cx: &App) -> (gpui_kit::Hsla, String) {
    let t = theme(cx);
    match &p.state {
        State::Stopped => (t.muted, "Stopped".into()),
        State::Starting => (t.warning, "Starting".into()),
        State::Running if p.ready => (t.success, "Running · ready".into()),
        State::Running => (t.warning, "Running".into()),
        State::Exited(Some(0)) => (t.muted, "Exited with 0".into()),
        State::Exited(Some(c)) => (t.error, format!("Exited with {c}")),
        State::Exited(None) => (t.muted, "Stopped by a signal".into()),
        State::Failed(e) => (t.error, format!("Failed: {e}")),
    }
}

/// What letting agents start a process means.
fn warning(e: &Row, cx: &App) -> String {
    let Ok(def) = &e.def else {
        return String::new();
    };
    let profiles: BTreeMap<String, Vec<String>> =
        Settings::get(cx).block(secrets::BLOCK).unwrap_or_default();
    let with = match &def.profile {
        Some(p) => format!(
            "with the env profile {p}: {}",
            profiles
                .get(p)
                .map_or("no variables".into(), |v| v.join(", "))
        ),
        None => "with no env profile".into(),
    };
    format!(
        "This lets agents run code as you: `{}` runs repo files an agent can edit, outside its \
            sandbox, {with}. Each start by an agent shows in its chat and in Events.",
        def.command
    )
}

impl Render for Processes {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let entries = self.entries(cx);
        if let Some(repo) = &self.repo {
            for e in &entries {
                if let Some(p) = procs::get(cx, repo, &e.name) {
                    (self.runs.entry(p.entity_id()))
                        .or_insert_with(|| cx.observe(&p, |_, _, cx| cx.notify()));
                }
            }
        }
        let loading = self.defs.as_ref().map(|d| d.read(cx));
        let note = match (&self.repo, loading) {
            (None, _) => Some("Select a repository".to_string()),
            _ if self.form.is_some() => None,
            (_, Some(r)) if r.value.is_none() => {
                Some(r.error.clone().unwrap_or("Loading processes…".into()))
            }
            _ if entries.is_empty() => Some("No processes yet: New defines one.".into()),
            _ => None,
        };
        let shown = self
            .selected
            .as_ref()
            .and_then(|s| entries.iter().find(|e| &e.name == s));
        let detail = match (&self.form, shown) {
            (Some(f), _) => Some(self.editor(f, cx)),
            (None, Some(e)) => Some(self.detail(e, cx)),
            (None, None) => None,
        };
        let bar = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(6.))
            .px(px(8.))
            .py(px(4.))
            .border_b_1()
            .border_color(theme(cx).line)
            .child(
                div()
                    .flex_1()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Processes"),
            )
            .when_some(self.error.clone(), |b, e| {
                b.child(
                    labeled("processes.error", e)
                        .text_size(text::SMALL)
                        .text_color(theme(cx).error),
                )
            })
            .when(self.repo.is_some(), |b| {
                b.child(button("processes.new", "New", true, cx, |this, w, cx| {
                    this.edit(None, w, cx)
                }))
            });
        div()
            .id("processes")
            .test_support()
            .flex()
            .flex_col()
            .size_full()
            .child(bar)
            .child(match note {
                Some(n) => div().p(px(10.)).child(muted(n, cx)).into_any_element(),
                None => div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .child(self.list(&entries, cx))
                    .children(detail)
                    .into_any_element(),
            })
    }
}
