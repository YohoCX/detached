use std::collections::{HashMap, HashSet};

use detached_core::api;
use detached_core::error::Result;
use detached_core::state::AppState;
use detached_core::types::{
    Job, JobAction, Pipeline, PipelineAction, PipelineStatus, Project, Provider, RunPipeline,
    Stage, Workflow,
};
use gpui_kit::base::TestSupportExt;
use gpui_kit::component::Sizable;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::{
    AnyElement, App, AppContext, ClickEvent, Context, Entity, FontWeight, Global,
    InteractiveElement, IntoElement, ParentElement, Render, Role, SharedString,
    StatefulInteractiveElement, Styled, Subscription, Task, Window, div, prelude::FluentBuilder,
    px,
};

use super::mr::setup_problem;
use super::{active, color, core, glyph, name, note, now, poll, setup_note, took};
use crate::cache::{self, Cache};
use crate::repos::{self, Section};
use crate::store::{DocRef, Location, Store};
use crate::ui::{self, Icon, Menu, radius, text, theme};
use crate::{nav, util};
use kernel::blocking;

/// For the session: the branch scope, and a Run Pipeline… request for the panel.
#[derive(Default)]
pub(super) struct Session {
    all: bool,
    pub run: bool,
}

impl Global for Session {}

/// Jobs Pipelines listed, with their pipeline id, by `job_key`: the job log's status and steps.
#[derive(Default)]
pub(super) struct Seen(pub HashMap<String, (Job, u64)>);

impl Global for Seen {}

pub(super) fn job_key(p: &Project, id: u64) -> String {
    format!("{}/{}#{id}", p.host, p.path)
}

/// An expanded pipeline's stages; None while loading.
type Jobs = Option<std::result::Result<Vec<Stage>, String>>;

struct RunForm {
    project: Project,
    github: bool,
    branch: Entity<InputState>,
    workflows: Vec<Workflow>,
    workflow: usize,
    vars: Vec<(Entity<InputState>, Entity<InputState>)>,
    _branch: Subscription,
}

/// A repo's pipelines; with `mr`, that MR's, without the branch scope and Run Pipeline….
pub struct Pipelines {
    /// The MR page's repo; None follows the current one.
    fixed: Option<String>,
    mr: Option<u64>,
    repo: Option<String>,
    key: String,
    head: String,
    /// The last action's error.
    note: Option<String>,
    busy: bool,
    github: HashSet<String>,
    /// Why the repo has no forge to list or run pipelines on.
    setup: Option<String>,
    run: Option<RunForm>,
    stages: HashMap<u64, Jobs>,
    poll: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl Pipelines {
    pub fn new(
        repo: Option<String>,
        mr: Option<u64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscriptions = vec![
            cx.observe(&Store::global(cx), |this, _, cx| this.sync(cx)),
            cx.observe_global::<Cache<Vec<Pipeline>>>(|_, cx| cx.notify()),
            cx.observe_global_in::<Session>(window, |this, window, cx| {
                this.sync(cx);
                this.take_run(window, cx);
            }),
            cx.observe_window_activation(window, |this, window, cx| {
                if window.is_window_active() {
                    this.load(true, cx);
                }
            }),
        ];
        let mut this = Pipelines {
            fixed: repo,
            mr,
            repo: None,
            key: String::new(),
            head: String::new(),
            note: None,
            busy: false,
            github: HashSet::new(),
            setup: None,
            run: None,
            stages: HashMap::new(),
            poll: None,
            _subscriptions: subscriptions,
        };
        this.sync(cx);
        this.take_run(window, cx);
        this
    }

    fn all(cx: &Context<Self>) -> bool {
        cx.try_global::<Session>().is_some_and(|s| s.all)
    }

    /// Follows the repo, the scope and the head: each change reloads, a new key collapses all.
    fn sync(&mut self, cx: &mut Context<Self>) {
        let s = Store::global(cx).read(cx);
        let repo = self.fixed.clone().or_else(|| s.current().map(String::from));
        let st = repo.as_deref().and_then(|r| s.status(r));
        let head = format!("{:?}", st.map(|s| (&s.head, &s.branch)));
        let at = repo.as_deref().unwrap_or_default();
        let key = format!("pipes {at} {} {}", self.mr.unwrap_or(0), Self::all(cx));
        if repo != self.repo {
            (self.run, self.setup) = (None, None);
            self.github.clear();
            if let Some(r) = repo.clone() {
                self.load_hosts(r, cx);
            }
        }
        if key != self.key {
            self.stages.clear();
        }
        if (&repo, &key, &head) != (&self.repo, &self.key, &self.head) {
            (self.repo, self.key, self.head) = (repo, key, head);
            self.load(false, cx);
        }
    }

    fn load_hosts(&mut self, repo: String, cx: &mut Context<Self>) {
        let (core, r) = (core(cx), repo.clone());
        let hosts = blocking::run(cx, move || hosts(&core, &r));
        cx.spawn(async move |this, cx| {
            let Ok((github, setup)) = hosts.await else {
                return;
            };
            this.update(cx, |this, cx| {
                if this.repo.as_ref() == Some(&repo) {
                    (this.github, this.setup) = (github, setup);
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// Refreshes the list, then the expanded pipelines still listed.
    fn load(&mut self, focus: bool, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.clone() else {
            return;
        };
        let (key, mr, all, core) = (self.key.clone(), self.mr, Self::all(cx), core(cx));
        let list = move || match mr {
            Some(n) => api::mr_pipelines(&core, &repo, n),
            None => api::list_pipelines(&core, &repo, all),
        };
        let list = cache::refresh(key.clone(), focus, list, cx);
        cx.spawn(async move |this, cx| {
            let Some(list) = list.await else { return };
            this.update(cx, |this, cx| {
                if this.key != key {
                    return;
                }
                this.stages.retain(|id, _| list.iter().any(|p| p.id == *id));
                let open: Vec<_> = list
                    .into_iter()
                    .filter(|p| this.stages.contains_key(&p.id))
                    .collect();
                for p in open {
                    this.jobs(p, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    fn jobs(&mut self, p: Pipeline, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.clone() else {
            return;
        };
        let (key, core, project, id) = (self.key.clone(), core(cx), p.project.clone(), p.id);
        let stages = blocking::run(cx, move || {
            api::pipeline_jobs(&core, &repo, &project, id).map_err(|e| e.to_string())
        });
        cx.spawn(async move |this, cx| {
            let stages = stages.await;
            this.update(cx, |this, cx| {
                if let Ok(stages) = &stages {
                    let seen = &mut cx.default_global::<Seen>().0;
                    for j in stages.iter().flat_map(|s| &s.jobs) {
                        seen.insert(job_key(&p.project, j.id), (j.clone(), id));
                    }
                }
                if this.key == key
                    && let Some(s) = this.stages.get_mut(&id)
                {
                    *s = Some(stages);
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    fn toggle(&mut self, p: &Pipeline, cx: &mut Context<Self>) {
        if self.stages.remove(&p.id).is_none() {
            self.stages.insert(p.id, None);
            self.jobs(p.clone(), cx);
        }
        cx.notify();
    }

    /// Runs `f` with every action disabled, its error in the note; `then` runs on success.
    fn act(
        &mut self,
        f: impl FnOnce(&AppState, &str) -> Result<()> + Send + 'static,
        then: fn(&mut Self),
        cx: &mut Context<Self>,
    ) {
        let Some(repo) = self.repo.clone() else {
            return;
        };
        (self.busy, self.note) = (true, None);
        cx.notify();
        let core = core(cx);
        let done = blocking::run(cx, move || f(&core, &repo));
        cx.spawn(async move |this, cx| {
            let r = done.await;
            this.update(cx, |this, cx| {
                this.busy = false;
                match r {
                    Ok(()) => then(this),
                    Err(e) => this.note = Some(e.to_string()),
                }
                this.load(false, cx);
            })
            .ok();
        })
        .detach();
    }

    fn take_run(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.mr.is_none() && cx.try_global::<Session>().is_some_and(|s| s.run) {
            cx.global_mut::<Session>().run = false;
            self.open_run(window, cx);
        }
    }

    fn open_run(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.clone() else {
            return;
        };
        let listed = cache::cached::<Vec<Pipeline>>(&self.key, cx)
            .and_then(|c| Some(c.value.as_ref()?.first()?.project.clone()));
        let st = Store::global(cx).read(cx).status(&repo);
        let branch = st.and_then(|s| s.branch.clone()).unwrap_or_default();
        let (core, r) = (core(cx), repo.clone());
        let form = blocking::run(cx, move || -> Result<_> {
            let (github, setup) = hosts(&core, &r)?;
            let project = match (listed, setup) {
                (Some(p), _) => p,
                // The panel says why in its body.
                (None, Some(_)) => return Ok(None),
                (None, None) => api::new_mr(&core, &r)?.source_project,
            };
            let github = github.contains(&project.host);
            let workflows = match github {
                true => api::workflows(&core, &r, &project)?,
                false => vec![],
            };
            Ok(Some((project, github, workflows)))
        });
        cx.spawn_in(window, async move |this, cx| {
            let form = form.await;
            cx.update(|window, cx| {
                this.update(cx, |this, cx| {
                    if this.repo.as_ref() != Some(&repo) {
                        return;
                    }
                    match form {
                        Ok(None) => {}
                        Ok(Some((project, github, workflows))) => {
                            let branch =
                                cx.new(|cx| InputState::new(window, cx).default_value(branch));
                            this.run = Some(RunForm {
                                _branch: cx
                                    .subscribe(&branch, |_, _, _: &InputEvent, cx| cx.notify()),
                                project,
                                github,
                                branch,
                                workflows,
                                workflow: 0,
                                vars: vec![],
                            });
                        }
                        Err(e) => this.note = Some(e.to_string()),
                    }
                    cx.notify();
                })
            })
            .ok();
        })
        .detach();
    }

    fn add_var(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let key = cx.new(|cx| InputState::new(window, cx).placeholder("Key"));
        let value = cx.new(|cx| InputState::new(window, cx).placeholder("Value"));
        if let Some(f) = &mut self.run {
            f.vars.push((key, value));
        }
        cx.notify();
    }

    fn pick_workflow(&mut self, e: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(f) = &self.run else { return };
        let this = cx.weak_entity();
        let items = f.workflows.iter().enumerate();
        let menu = items.fold(Menu::default(), |menu, (i, w)| {
            let this = this.clone();
            menu.item(w.name.clone(), move |_, cx| {
                this.update(cx, |this, cx| {
                    if let Some(f) = &mut this.run {
                        f.workflow = i;
                    }
                    cx.notify();
                })
                .ok();
            })
        });
        menu.show(e.position(), window, cx);
    }

    fn start(&mut self, cx: &mut Context<Self>) {
        let Some(f) = &self.run else { return };
        let value = |s: &Entity<InputState>| s.read(cx).value().to_string();
        let variables = f.vars.iter().map(|(k, v)| (value(k), value(v)));
        let input = RunPipeline {
            project: f.project.clone(),
            branch: value(&f.branch).trim().into(),
            workflow: f
                .workflows
                .get(f.workflow)
                .filter(|_| f.github)
                .map(|w| w.id.clone()),
            variables: variables.filter(|(k, _)| !k.trim().is_empty()).collect(),
        };
        let run =
            move |core: &AppState, repo: &str| api::run_pipeline(core, repo, &input).map(drop);
        self.act(run, |this| this.run = None, cx);
    }

    fn pipeline_action(
        &mut self,
        project: Project,
        id: u64,
        a: PipelineAction,
        cx: &mut Context<Self>,
    ) {
        self.act(
            move |core, repo| api::pipeline_action(core, repo, &project, id, a),
            |_| {},
            cx,
        );
    }

    fn job_action(&mut self, project: Project, id: u64, a: JobAction, cx: &mut Context<Self>) {
        self.act(
            move |core, repo| api::job_action(core, repo, &project, id, a),
            |_| {},
            cx,
        );
    }

    /// A small pill; while busy it shows disabled and does nothing.
    fn act_button<F: Fn(&mut Self, &mut Context<Self>) + 'static>(
        &self,
        id: &'static str,
        label: &'static str,
        run: F,
        cx: &Context<Self>,
    ) -> impl IntoElement + use<F> {
        let t = theme(cx);
        ui::pressable(id)
            .test_support()
            .role(Role::Button)
            .aria_label(label)
            .h(px(22.))
            .px(px(8.))
            .border_1()
            .border_color(t.line)
            .rounded(radius::PILL)
            .bg(t.input)
            .text_size(text::LABEL)
            .font_weight(FontWeight::BOLD)
            .when(self.busy, |b| b.opacity(0.5))
            .when(!self.busy, |b| {
                b.on_click(cx.listener(move |this, _, _, cx| run(this, cx)))
            })
            .child(label)
    }

    fn bar(&self, note: Option<String>, off: bool, cx: &Context<Self>) -> impl IntoElement + use<> {
        let t = theme(cx);
        let scope = [(false, "Current branch"), (true, "All branches")];
        let note = note.unwrap_or_default();
        let tip = note.clone();
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(6.))
            .px(px(10.))
            .py(px(6.))
            .border_b_1()
            .border_color(t.line)
            .text_size(text::SMALL)
            .when(self.mr.is_none(), |d| {
                d.child(ui::segmented(
                    "pipelines.scope",
                    &scope,
                    Self::all(cx),
                    |all, _, cx| cx.default_global::<Session>().all = all,
                    cx,
                ))
            })
            .child(
                div()
                    .id("pipelines.note")
                    .test_support()
                    .aria_label(note.clone())
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_color(t.error)
                    .when(!note.is_empty(), |d| {
                        d.tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                    })
                    .child(note),
            )
            .when(self.mr.is_none(), |d| {
                let off = off || self.busy;
                let run = ui::button("pipelines.run", "Run Pipeline…", cx)
                    .when(off, |b| b.opacity(0.5))
                    .when(!off, |b| {
                        b.on_click(cx.listener(|this, _, window, cx| this.open_run(window, cx)))
                    });
                d.child(ui::disabled(run, off).test_support())
            })
            .child(
                ui::icon_button("pipelines.refresh", Icon::Refresh, cx)
                    .test_support()
                    .tooltip(|window, cx| Tooltip::new("Refresh").build(window, cx))
                    .on_click(cx.listener(|this, _, _, cx| this.load(false, cx))),
            )
    }

    fn form(&self, f: &RunForm, cx: &Context<Self>) -> impl IntoElement + use<> {
        let t = theme(cx);
        let blank = f.branch.read(cx).value().trim().is_empty();
        let can_run = !self.busy && !blank && (!f.github || !f.workflows.is_empty());
        let field = |label: &'static str, e: AnyElement| {
            div()
                .flex()
                .items_center()
                .gap(px(4.))
                .child(label)
                .child(e)
        };
        let workflow: SharedString = f
            .workflows
            .get(f.workflow)
            .map_or("".into(), |w| w.name.clone().into());
        let vars = f.vars.iter().enumerate().map(|(i, (k, v))| {
            div()
                .flex()
                .items_center()
                .gap(px(4.))
                .child(Input::new(k).id(("run.key", i)).small().w(px(120.)))
                .child(Input::new(v).id(("run.value", i)).small().w(px(120.)))
                .child(
                    ui::icon_button(("run.remove", i), Icon::Close.size(px(12.)), cx)
                        .test_support()
                        .tooltip(|window, cx| Tooltip::new("Remove").build(window, cx))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(f) = &mut this.run {
                                f.vars.remove(i);
                            }
                            cx.notify();
                        })),
                )
        });
        div()
            .flex()
            .flex_none()
            .flex_wrap()
            .items_center()
            .gap(px(6.))
            .px(px(10.))
            .py(px(6.))
            .border_b_1()
            .border_color(t.line)
            .text_size(text::SMALL)
            .child(field(
                "Branch",
                Input::new(&f.branch)
                    .id("run.branch")
                    .small()
                    .w(px(160.))
                    .into_any_element(),
            ))
            .when(f.github, |d| {
                let pick = ui::pressable("run.workflow")
                    .test_support()
                    .role(Role::ComboBox)
                    .aria_value(workflow.clone())
                    .gap(px(4.))
                    .h(px(24.))
                    .px(px(8.))
                    .border_1()
                    .border_color(t.line)
                    .rounded(radius::ITEM)
                    .bg(t.input)
                    .on_click(cx.listener(|this, e, window, cx| this.pick_workflow(e, window, cx)))
                    .child(workflow)
                    .child(Icon::Down.size(px(12.)));
                d.child(field("Workflow", pick.into_any_element()))
            })
            .children(vars)
            .child(
                ui::icon_button("run.add", Icon::Plus.size(px(12.)), cx)
                    .test_support()
                    .child(if f.github { "Input" } else { "Variable" })
                    .on_click(cx.listener(|this, _, window, cx| this.add_var(window, cx))),
            )
            .child(div().flex_1())
            .child(
                ui::button("run.start", "Run", cx)
                    .test_support()
                    .bg(t.primary)
                    .text_color(t.primary_fg)
                    .when(!can_run, |b| b.opacity(0.5))
                    .when(can_run, |b| {
                        b.on_click(cx.listener(|this, _, _, cx| this.start(cx)))
                    }),
            )
            .child(
                ui::button("run.cancel", "Cancel", cx)
                    .test_support()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.run = None;
                        cx.notify();
                    })),
            )
    }

    fn row(&self, p: &Pipeline, cx: &Context<Self>) -> impl IntoElement + use<> {
        let t = theme(cx);
        let sha = p.sha.get(..8).unwrap_or(&p.sha).to_string();
        let end = p.finished.unwrap_or_else(now);
        let took = p
            .started
            .map(|s| format!(" · {}", took(Some((end - s).max(0) as u64))));
        let meta = format!(
            "{} · {}{}",
            p.source,
            util::when(p.created),
            took.unwrap_or_default()
        );
        let failed = p.status == PipelineStatus::Failed;
        let retry_failed = self.github.contains(&p.project.host) && p.retryable && failed;
        let button = |id, label, a: PipelineAction| {
            let (project, n) = (p.project.clone(), p.id);
            let run = move |this: &mut Self, cx: &mut Context<Self>| {
                this.pipeline_action(project.clone(), n, a.clone(), cx)
            };
            self.act_button(id, label, run, cx)
        };
        let p1 = p.clone();
        div()
            .id(("pipeline", p.id))
            .test_support()
            .role(Role::ListItem)
            .aria_label(format!(
                "{}#{} {} {sha} {meta}",
                glyph(&p.status),
                p.id,
                p.ref_
            ))
            .flex()
            .items_center()
            .gap(px(2.))
            .h(px(28.))
            .px(px(8.))
            .hover(|s| s.bg(t.hover))
            .child(
                div()
                    .id("toggle")
                    .test_support()
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .items_center()
                    .gap(px(8.))
                    .px(px(4.))
                    .on_click(cx.listener(move |this, _, _, cx| this.toggle(&p1, cx)))
                    .child(match self.stages.contains_key(&p.id) {
                        true => Icon::Down.size(px(12.)),
                        false => Icon::Chevron.size(px(12.)),
                    })
                    .child(status(&p.status, cx))
                    .child(
                        div()
                            .font_weight(FontWeight::BOLD)
                            .child(format!("#{}", p.id)),
                    )
                    .child(div().min_w_0().truncate().child(p.ref_.clone()))
                    .child(
                        div()
                            .font_family(theme::mono(cx))
                            .text_color(t.muted)
                            .child(sha),
                    )
                    .child(div().min_w_0().truncate().text_color(t.muted).child(meta)),
            )
            .when(retry_failed, |d| {
                d.child(button(
                    "retry-failed",
                    "Retry Failed",
                    PipelineAction::RetryFailed,
                ))
            })
            .when(p.retryable, |d| {
                d.child(button("retry", "Retry", PipelineAction::Retry))
            })
            .when(p.cancellable, |d| {
                d.child(button("cancel", "Cancel", PipelineAction::Cancel))
            })
    }

    fn job(&self, p: &Pipeline, j: &Job, cx: &Context<Self>) -> impl IntoElement + use<> {
        let t = theme(cx);
        let duration = took(j.duration);
        let repo = self.repo.clone().unwrap_or_default();
        let doc = DocRef::Job {
            repo: repo.clone(),
            project: p.project.clone(),
            id: j.id,
            name: j.name.clone(),
        };
        let loc = Location {
            repo,
            rev: None,
            path: String::new(),
            line: None,
            doc: Some(doc),
        };
        let button = |id, label, a: JobAction| {
            let (project, n) = (p.project.clone(), j.id);
            let run = move |this: &mut Self, cx: &mut Context<Self>| {
                this.job_action(project.clone(), n, a.clone(), cx)
            };
            self.act_button(id, label, run, cx)
        };
        div()
            .id(("job", j.id))
            .test_support()
            .role(Role::ListItem)
            .aria_label(format!("{}{} {duration}", glyph(&j.status), j.name))
            .flex()
            .items_center()
            .gap(px(2.))
            .h(px(28.))
            .pl(px(48.))
            .pr(px(8.))
            .hover(|s| s.bg(t.hover))
            .child(
                div()
                    .id("open")
                    .test_support()
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .items_center()
                    .gap(px(8.))
                    .px(px(4.))
                    .on_click(move |_, _, cx| nav::open(cx, loc.clone()))
                    .child(status(&j.status, cx))
                    .child(div().min_w_0().truncate().child(j.name.clone()))
                    .when(j.allow_failure, |d| {
                        d.child(
                            div()
                                .px(px(5.))
                                .border_1()
                                .border_color(t.line)
                                .rounded(radius::KBD)
                                .text_size(text::LABEL)
                                .text_color(t.warning)
                                .whitespace_nowrap()
                                .child("allowed to fail"),
                        )
                    })
                    .child(div().text_color(t.muted).child(duration)),
            )
            .when(j.playable, |d| {
                d.child(button("play", "Play", JobAction::Play))
            })
            .when(j.retryable, |d| {
                d.child(button("retry", "Retry", JobAction::Retry))
            })
            .when(j.cancellable, |d| {
                d.child(button("cancel", "Cancel", JobAction::Cancel))
            })
    }

    fn expanded(&self, p: &Pipeline, jobs: &Jobs, cx: &Context<Self>) -> impl IntoElement + use<> {
        let t = theme(cx);
        let sub = || {
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .h(px(24.))
                .pl(px(34.))
                .text_size(text::SMALL)
        };
        let rows: Vec<AnyElement> = match jobs {
            None => vec![
                sub()
                    .text_color(t.muted)
                    .child("Loading…")
                    .into_any_element(),
            ],
            Some(Err(e)) => vec![
                sub()
                    .text_color(t.error)
                    .child(e.clone())
                    .into_any_element(),
            ],
            Some(Ok(stages)) => stages
                .iter()
                .enumerate()
                .flat_map(|(i, g)| {
                    let stage = sub()
                        .id(("stage", i))
                        .test_support()
                        .aria_label(format!("{}{}", glyph(&g.status), g.name))
                        .text_color(t.muted)
                        .font_weight(FontWeight::BOLD)
                        .child(
                            div()
                                .text_color(color(&g.status, cx))
                                .child(glyph(&g.status)),
                        )
                        .child(g.name.clone());
                    let jobs = g.jobs.iter().map(|j| self.job(p, j, cx).into_any_element());
                    std::iter::once(stage.into_any_element()).chain(jobs)
                })
                .collect(),
        };
        div().id(("jobs", p.id)).children(rows)
    }
}

impl Render for Pipelines {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let cached = cache::cached::<Vec<Pipeline>>(&self.key, cx);
        let (list, error) = (
            cached.and_then(|c| c.value.clone()),
            cached.and_then(|c| c.error.clone()),
        );
        let jobs = self.stages.values().flatten().flatten().flatten();
        let running = list.iter().flatten().any(|p| active(&p.status))
            || jobs.flat_map(|s| &s.jobs).any(|j| active(&j.status));
        if running != self.poll.is_some() {
            self.poll = running.then(|| poll(5, |this, cx| this.load(false, cx), window, cx));
        }
        let t = theme(cx);
        if self.repo.is_none() {
            return note("pipelines.note", "No repository", t.muted);
        }
        let setup = self.setup.clone().filter(|_| list.is_none());
        let body = match (&list, error.clone(), setup.clone()) {
            (None, _, Some(s)) => setup_note("pipelines.setup", "pipelines.settings", s, cx),
            (None, Some(e), None) => {
                let settings = e.ends_with("in Settings");
                div()
                    .id("pipelines.error")
                    .test_support()
                    .aria_label(e.clone())
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(8.))
                    .px(px(16.))
                    .py(px(12.))
                    .text_color(t.error)
                    .child(e)
                    .when(settings, |d| {
                        d.child(
                            ui::button("pipelines.settings", "Open Settings", cx)
                                .test_support()
                                .text_color(t.fg)
                                .on_click(|_, _, cx| repos::open_settings(cx, Section::Repos)),
                        )
                    })
                    .into_any_element()
            }
            (None, None, None) => note("pipelines.note", "Loading…", t.muted),
            (Some(l), ..) if l.is_empty() => note("pipelines.note", "No pipelines", t.muted),
            (Some(l), ..) => div()
                .children(l.iter().flat_map(|p| {
                    let row = self.row(p, cx).into_any_element();
                    let jobs = self
                        .stages
                        .get(&p.id)
                        .map(|j| self.expanded(p, j, cx).into_any_element());
                    std::iter::once(row).chain(jobs)
                }))
                .into_any_element(),
        };
        let note = self.note.clone().or_else(|| list.and(error));
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(self.bar(note, setup.is_some(), cx))
            .children(self.run.as_ref().map(|f| self.form(f, cx)))
            .child(
                div()
                    .id("pipelines.body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .py(px(4.))
                    .child(body),
            )
            .into_any_element()
    }
}

/// Hosts whose account is GitHub's, which alone retry failed jobs and dispatch workflows, and
/// why the repo has no forge.
fn hosts(core: &AppState, repo: &str) -> Result<(HashSet<String>, Option<String>)> {
    let github = |p: &Provider| matches!(p, Provider::Github | Provider::GithubEnterprise);
    let accounts = api::list_accounts(core)?
        .into_iter()
        .filter(|a| github(&a.provider));
    let ids: HashSet<_> = accounts.map(|a| a.id).collect();
    let hosts = api::forge_hosts(core, repo)?;
    let setup = setup_problem(&hosts);
    let ours = |id: &Option<String>| id.as_ref().is_some_and(|id| ids.contains(id));
    let github = hosts.hosts.into_iter().filter(|h| ours(&h.account_id));
    Ok((github.map(|h| h.host).collect(), setup))
}

/// A status glyph in its color, the status as its tooltip.
pub(super) fn status(s: &PipelineStatus, cx: &App) -> impl IntoElement + use<> {
    let tip = name(s);
    div()
        .id("status")
        .text_color(color(s, cx))
        .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
        .child(glyph(s))
}
