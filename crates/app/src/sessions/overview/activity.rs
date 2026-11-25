use std::collections::{HashMap, HashSet};

use detached_core::api::SessionBase;
use detached_core::council::record::{Of, Subject};
use detached_core::council::track::Fate;
use detached_core::sessions::MAIN;
use detached_core::types::{
    MergeRequest, MrState, Pipeline, PipelineStatus, Project, Provider, RepoInfo,
};
use gpui_kit::base::TestSupportExt;
use gpui_kit::{
    AnyElement, App, Context, Entity, EntityId, FontWeight, Hsla, IntoElement, ParentElement, Role,
    SharedString, StatefulInteractiveElement, Styled, Subscription, Window, div,
};
use kernel::council::{self, fixes, live};
use kernel::doc::Doc;
use kernel::repo::Repo;
use kernel::resource::{Interest, Resource};
use kernel::session::{Answer, Item, Session, State};
use kernel::workspace::Workspace;
use kernel::{forge, proc, workset};

use super::{Overview, status};
use crate::ui::{self, GAP, Icon, text, theme};
use crate::{chat, dock, util};

pub(super) struct OverviewActivity {
    repos: HashMap<String, MonitoredRepo>,
    observed: HashMap<EntityId, Subscription>,
    _registries: [Subscription; 4],
}

#[derive(Default)]
pub(super) struct MonitoredRepo {
    rounds: Option<Interest<council::Rounds>>,
    base: Option<Base>,
    branch: Option<TrackedBranch>,
    pipelines: Option<MrPipelines>,
}

struct Base {
    repo: String,
    reference: Option<String>,
    comparison: Interest<SessionBase>,
}

struct TrackedBranch {
    key: (String, String),
    mr: Interest<Option<MergeRequest>>,
    latest: Interest<Option<Pipeline>>,
}

struct MrPipelines {
    project: Project,
    number: u64,
    head: String,
    list: Interest<Vec<Pipeline>>,
}

impl OverviewActivity {
    pub(super) fn new(cx: &mut Context<Overview>) -> Self {
        Self {
            repos: HashMap::new(),
            observed: HashMap::new(),
            _registries: [
                chat::observe_open(cx),
                proc::observe_runs(cx),
                live::observe_runs(cx),
                fixes::observe_runs(cx),
            ],
        }
    }

    pub(super) fn rows(
        &mut self,
        cx: &mut Context<Overview>,
    ) -> (Vec<AnyElement>, Vec<AnyElement>, Vec<AnyElement>) {
        let workspace = Workspace::get(cx);
        let session = workspace.session_id();
        let Some(workspace) = workspace
            .id()
            .filter(|_| session != MAIN && !workspace.archive())
        else {
            self.repos.clear();
            self.observed.clear();
            return (vec![], vec![], vec![]);
        };
        let repos = Workspace::get(cx).session_repos().to_vec();
        let in_session = |repo: &str| repos.iter().find(|r| r.path == repo);
        let chats = chat::open(cx)
            .into_iter()
            .filter(|(r, _, chat)| {
                in_session(r).is_some() && chat.read(cx).owned_by(workspace, session)
            })
            .collect::<Vec<_>>();
        let councils = live::all(cx);
        let fixes = fixes::all(cx).collect::<Vec<_>>();
        let mut seen = HashSet::new();
        let (mut needs, mut running, mut statuses) = (vec![], vec![], vec![]);
        let colors = theme(cx);
        let (warning, active, error) = (colors.warning, colors.accent, colors.error);
        for (repo, n, chat) in chats {
            self.watch(&chat, &mut seen, cx);
            let chat = chat.read(cx);
            let Some(s) = chat.current().map(|s| s.read(cx)) else {
                continue;
            };
            let (rows, label, color) = if let Some(label) = waiting(s) {
                (&mut needs, label, warning)
            } else if s.state == State::Running {
                (&mut running, "Agent running", active)
            } else {
                continue;
            };
            let topic = chat
                .topic(cx)
                .map_or_else(|| "New chat".into(), util::first_line);
            let detail = format!(
                "{} · {} · {topic}",
                chat::label(&repo, cx),
                chat.provider.name
            );
            rows.push(row(
                format!("overview.chat.{repo}.{n}"),
                label.into(),
                detail,
                color,
                move |_, cx| chat::foreground(repo.clone(), n, cx),
                cx,
            ));
        }
        for council in councils {
            let c = council.read(cx);
            if in_session(&c.repo).is_none()
                || !matches!(c.phase, live::Phase::Preparing | live::Phase::Running)
            {
                continue;
            }
            self.watch(&council, &mut seen, cx);
            let c = council.read(cx);
            let members: Vec<_> = c
                .live
                .iter()
                .filter(|m| m.status == live::Status::Running)
                .filter_map(|m| Some((m.name.clone(), m.session.clone()?)))
                .collect();
            let doc = Doc::Council {
                repo: c.repo.clone(),
                task: c.task.clone(),
                round: c.run.clone(),
            };
            let detail = format!("{} · Council · {}", chat::label(&c.repo, cx), c.title());
            for (name, s) in members {
                self.watch(&s, &mut seen, cx);
                if !running_turn(s.read(cx)) {
                    continue;
                }
                let doc = doc.clone();
                running.push(row(
                    format!("overview.councilAgent.{}.{name}", s.entity_id().as_u64()),
                    format!("{name} is running"),
                    detail.clone(),
                    active,
                    move |w, cx| dock::open_doc(doc.clone(), true, w, cx),
                    cx,
                ));
            }
        }
        for fix in fixes {
            let f = fix.read(cx);
            if in_session(&f.repo).is_none()
                || !matches!(f.phase, fixes::Phase::Making | fixes::Phase::Working)
            {
                continue;
            }
            self.watch(&fix, &mut seen, cx);
            let Some(s) = fix.read(cx).session.clone() else {
                continue;
            };
            self.watch(&s, &mut seen, cx);
            let f = fix.read(cx);
            if running_turn(s.read(cx)) {
                running.push(finding_row(
                    [&f.repo, &f.task, &f.run, &f.finding],
                    "Fix agent running",
                    f.task.clone(),
                    active,
                    cx,
                ));
            }
        }
        self.repos.retain(|r, _| in_session(r).is_some());
        for (index, repo) in repos.iter().enumerate() {
            let processes = proc::runs(cx, &repo.path).collect::<Vec<_>>();
            for p in processes {
                self.watch(&p, &mut seen, cx);
                let p = p.read(cx);
                if p.state == proc::State::Running {
                    let (path, name) = (p.repo.clone(), p.name.clone());
                    running.push(row(
                        format!("overview.process.{}.{}", p.repo, p.name),
                        p.name.clone(),
                        format!("{} · Process running", repo.name),
                        active,
                        move |w, cx| crate::processes::show_run(path.clone(), name.clone(), w, cx),
                        cx,
                    ));
                }
            }
            let monitor = self.repos.entry(repo.path.clone()).or_default();
            monitor.follow(repo, cx);
            statuses.push(status::row(index, repo, monitor, cx));
            monitor.findings(repo, &mut needs, warning, cx);
            for (mr, p) in monitor.failed(cx) {
                let Ok(url) = url::Url::parse(&p.web_url) else {
                    continue;
                };
                if !matches!(url.scheme(), "http" | "https") || !url.has_host() {
                    continue;
                }
                let repo = repo.path.clone();
                let (number, pipeline) = (mr.number, p.id);
                let sign = if matches!(mr.provider, Provider::Gitlab | Provider::GitlabSelfHosted) {
                    '!'
                } else {
                    '#'
                };
                needs.push(row(
                    format!("overview.pipeline.{repo}.{number}.{pipeline}"),
                    format!("Pipeline #{pipeline} failed"),
                    format!(
                        "{} · {sign}{number} {}",
                        chat::label(&repo, cx),
                        util::first_line(&mr.title)
                    ),
                    error,
                    move |_, cx| cx.open_url(url.as_str()),
                    cx,
                ));
            }
        }
        self.observed.retain(|id, _| seen.contains(id));
        (needs, running, statuses)
    }

    fn watch<T: 'static>(
        &mut self,
        entity: &Entity<T>,
        seen: &mut HashSet<EntityId>,
        cx: &mut Context<Overview>,
    ) {
        let id = entity.entity_id();
        seen.insert(id);
        self.observed
            .entry(id)
            .or_insert_with(|| cx.observe(entity, |_, _, cx| cx.notify()));
    }
}

impl MonitoredRepo {
    fn follow(&mut self, repo: &RepoInfo, cx: &mut Context<Overview>) {
        if self.rounds.is_none() {
            self.rounds = Some(Interest::view(council::rounds(cx, &repo.path), cx));
        }
        let workspace = Workspace::get(cx);
        let member = workspace
            .sessions()
            .iter()
            .find(|set| set.id == workspace.session_id())
            .and_then(|set| set.members.iter().find(|m| m.path == repo.path));
        match member {
            Some(m)
                if self
                    .base
                    .as_ref()
                    .is_none_or(|b| b.repo != m.repo || b.reference != m.base) =>
            {
                let m = m.clone();
                self.base = Some(Base {
                    comparison: Interest::view(workset::compare_base(cx, &m), cx),
                    repo: m.repo,
                    reference: m.base,
                });
            }
            Some(_) => {}
            None => self.base = None,
        }
        let key = (|| {
            let status = Repo::get(cx, &repo.path)?.status()?;
            let (branch, upstream) = (status.branch.as_ref()?, status.upstream.as_ref()?);
            Some((
                format!("{branch}\0{upstream}"),
                format!("{:?} {:?}", status.head, status.branch),
            ))
        })();
        if self.branch.as_ref().map(|b| &b.key) != key.as_ref() {
            self.branch = key.map(|key| {
                let mr = forge::current_mr(cx, &repo.path, &key.0);
                let latest = forge::latest_pipeline(cx, &repo.path, &key.1);
                TrackedBranch {
                    key,
                    mr: Interest::view(mr, cx),
                    latest: Interest::view(latest, cx),
                }
            });
            self.pipelines = None;
        }
        self.pipelines = match self.mr(cx).flatten() {
            Some(mr)
                if self.pipelines.as_ref().is_none_or(|p| {
                    p.number != mr.number || p.project != mr.target_project || p.head != mr.head_sha
                }) =>
            {
                let (number, project, head) =
                    (mr.number, mr.target_project.clone(), mr.head_sha.clone());
                let resource = forge::pipelines(cx, &repo.path, Some(number), false);
                Some(MrPipelines {
                    project,
                    number,
                    head,
                    list: Interest::view(resource, cx),
                })
            }
            Some(_) => self.pipelines.take(),
            None => None,
        };
    }

    pub(super) fn base<'a>(&self, cx: &'a App) -> Option<&'a Resource<SessionBase>> {
        self.base.as_ref()?.comparison.loaded(cx)
    }

    pub(super) fn mr<'a>(&self, cx: &'a App) -> Option<Option<&'a MergeRequest>> {
        let r = self.branch.as_ref()?.mr.loaded(cx)?;
        let value = r.value.as_ref().filter(|_| r.error.is_none())?;
        Some(value.as_ref().filter(|m| m.state == MrState::Open))
    }

    pub(super) fn latest<'a>(&self, cx: &'a App) -> Option<&'a Pipeline> {
        self.branch
            .as_ref()?
            .latest
            .read(cx)
            .value
            .as_ref()?
            .as_ref()
    }

    fn failed<'a>(&self, cx: &'a App) -> Vec<(&'a MergeRequest, &'a Pipeline)> {
        let Some(mr) = self.mr(cx).flatten() else {
            return Vec::new();
        };
        let Some(runs) = self
            .pipelines
            .as_ref()
            .and_then(|p| p.list.loaded(cx))
            .filter(|r| r.error.is_none())
            .and_then(|r| r.value.as_ref())
        else {
            return Vec::new();
        };
        failed_pipelines(&mr.head_sha, &mr.source_project, &mr.target_project, runs)
            .into_iter()
            .map(|p| (mr, p))
            .collect()
    }

    fn findings(&self, repo: &RepoInfo, needs: &mut Vec<AnyElement>, color: Hsla, cx: &App) {
        let Some(r) = self.rounds.as_ref().and_then(|rounds| rounds.loaded(cx)) else {
            return;
        };
        let Some(rounds) = r.value.as_ref().filter(|_| r.error.is_none()) else {
            return;
        };
        let branch = Repo::get(cx, &repo.path)
            .and_then(Repo::status)
            .and_then(|s| s.branch.as_deref());
        let mr = self.mr(cx).flatten();
        for (task, entry) in &rounds.tasks {
            let Some(track) = entry.track.as_ref().filter(|t| !t.running) else {
                continue;
            };
            let Some(round) = track.rounds.last() else {
                continue;
            };
            let relevant = |round: &String| {
                rounds
                    .records
                    .get(&(task.clone(), round.clone()))
                    .and_then(|r| r.started.as_ref())
                    .is_some_and(|s| on_subject(&s.subject, &repo.path, branch, mr))
            };
            if !relevant(round) {
                continue;
            }
            for thread in track
                .gating()
                .filter(|t| t.fate == Fate::Open && track.complete)
                .chain(track.threads.iter().filter(|t| t.fate == Fate::FixReady))
            {
                let mut raised =
                    track
                        .rounds
                        .iter()
                        .zip(&thread.marks)
                        .rev()
                        .filter_map(|(round, mark)| {
                            let id = mark.as_ref()?.finding()?;
                            relevant(round).then_some((round, id))
                        });
                let point = match thread.fate {
                    Fate::FixReady => raised.find(|(round, id)| {
                        entry
                            .fixing
                            .get(&(round.to_string(), id.to_string()))
                            .and_then(|s| s.check.as_ref())
                            .is_some_and(|c| c.passed == Some(true))
                    }),
                    _ => raised.next(),
                };
                let Some((round, id)) = point else { continue };
                if fixes::find(cx, [&repo.path, task, round, id]).is_some_and(|f| {
                    matches!(
                        f.read(cx).phase,
                        fixes::Phase::Making | fixes::Phase::Working | fixes::Phase::Checking
                    )
                }) {
                    continue;
                }
                let label = if thread.fate == Fate::FixReady {
                    "Council fix ready"
                } else {
                    "Council review blocked"
                };
                needs.push(finding_row(
                    [&repo.path, task, round, id],
                    label,
                    thread.title.clone(),
                    color,
                    cx,
                ));
            }
        }
    }
}

fn running_turn(session: &Session) -> bool {
    session.state == State::Running && waiting(session).is_none()
}

fn waiting(session: &Session) -> Option<&'static str> {
    session.items.iter().rev().find_map(|item| match item {
        Item::Ask {
            answer: Answer::Waiting,
            questions,
            ..
        } => Some(if questions.is_empty() {
            "Permission requested"
        } else {
            "Agent has a question"
        }),
        _ => None,
    })
}

fn on_subject(s: &Subject, repo: &str, branch: Option<&str>, mr: Option<&MergeRequest>) -> bool {
    s.repo == repo
        && match &s.of {
            Of::Branch => !s.remote && branch == Some(&s.branch),
            Of::Mr { number, url, .. } => {
                mr.is_some_and(|m| m.number == *number && m.web_url == *url)
            }
            _ => false,
        }
}

fn failed_pipelines<'a>(
    head: &str,
    source: &Project,
    target: &Project,
    pipelines: &'a [Pipeline],
) -> Vec<&'a Pipeline> {
    let mut newest = HashMap::new();
    for p in pipelines.iter().filter(|p| {
        !head.is_empty() && p.sha == head && (&p.project == target || &p.project == source)
    }) {
        // GitHub runs each workflow independently. GitLab returns whole MR pipelines.
        let workflow = p.workflow_id.map(|id| (&p.project.path, id));
        let run = newest.entry(workflow).or_insert(p);
        if (p.created, p.id) > (run.created, run.id) {
            *run = p;
        }
    }
    let mut failed = newest
        .into_values()
        .filter(|p| p.status == PipelineStatus::Failed)
        .collect::<Vec<_>>();
    failed.sort_by_key(|p| std::cmp::Reverse((p.created, p.id)));
    failed
}

fn finding_row(
    [repo, task, round, id]: [&str; 4],
    label: &str,
    title: String,
    color: Hsla,
    cx: &App,
) -> AnyElement {
    let detail = format!("{} · {title}", chat::label(repo, cx));
    let key = format!("overview.councilFinding.{repo}.{task}.{round}.{id}");
    let [repo, task, round, id] = [repo, task, round, id].map(str::to_owned);
    row(
        key,
        label.into(),
        detail,
        color,
        move |_, cx| crate::council::show_finding(&repo, &task, &round, Some(&id), cx),
        cx,
    )
}

fn row(
    id: String,
    label: String,
    detail: String,
    color: Hsla,
    open: impl Fn(&mut Window, &mut App) + 'static,
    cx: &App,
) -> AnyElement {
    let t = theme(cx);
    ui::row(SharedString::from(id), false, cx)
        .test_support()
        .role(Role::Button)
        .aria_label(format!("{label} · {detail}"))
        .cursor_pointer()
        .min_w_0()
        .py(GAP / 2.)
        .child(ui::dot(color))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .text_size(text::HEADING)
                        .font_weight(FontWeight::MEDIUM)
                        .child(label),
                )
                .child(
                    div()
                        .text_size(text::BODY)
                        .text_color(t.muted)
                        .child(detail),
                ),
        )
        .child(
            div()
                .flex_none()
                .text_color(t.muted)
                .child(Icon::Chevron.size(text::SMALL)),
        )
        .on_click(move |_, w, cx| open(w, cx))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_ci_requires_the_mr_head_and_project_and_clears_after_a_newer_run() {
        let project = |path: &str| Project {
            host: "forge.example".into(),
            path: path.into(),
        };
        let (source, target, unrelated) = (
            project("fork/app"),
            project("org/app"),
            project("other/app"),
        );
        let pipeline = |id, project: &Project, sha: &str, status| Pipeline {
            id,
            workflow_id: None,
            project: project.clone(),
            status,
            ref_: "topic".into(),
            sha: sha.into(),
            source: "push".into(),
            created: 0,
            started: None,
            finished: None,
            web_url: format!("https://forge.example/pipelines/{id}"),
            cancellable: false,
            retryable: true,
        };
        let mut pipelines = vec![
            pipeline(1, &target, "before", PipelineStatus::Failed),
            pipeline(2, &unrelated, "head", PipelineStatus::Failed),
        ];
        let failed = |pipelines: &[Pipeline]| {
            failed_pipelines("head", &source, &target, pipelines)
                .into_iter()
                .map(|p| p.id)
                .collect::<Vec<_>>()
        };
        assert!(failed(&pipelines).is_empty());
        pipelines.push(pipeline(3, &source, "head", PipelineStatus::Failed));
        assert_eq!(failed(&pipelines), [3]);
        pipelines.push(pipeline(4, &target, "head", PipelineStatus::Success));
        assert!(failed(&pipelines).is_empty());
        pipelines[2].workflow_id = Some(21);
        pipelines[3].workflow_id = Some(57);
        assert_eq!(failed(&pipelines), [3]);
        pipelines.push(Pipeline {
            workflow_id: Some(21),
            ..pipeline(5, &source, "head", PipelineStatus::Success)
        });
        assert!(failed(&pipelines).is_empty());
    }

    #[test]
    fn council_subject_must_use_the_session_checkout_and_branch() {
        let mut s = Subject {
            repo: "/worktrees/topic".into(),
            branch: "topic".into(),
            remote: false,
            base: "main".into(),
            base_sha: "base".into(),
            head_sha: "head".into(),
            of: Of::Branch,
        };
        assert!(on_subject(&s, "/worktrees/topic", Some("topic"), None));
        assert!(!on_subject(&s, "/main", Some("topic"), None));
        assert!(!on_subject(&s, "/worktrees/topic", Some("elsewhere"), None));
        s.remote = true;
        assert!(!on_subject(&s, "/worktrees/topic", Some("topic"), None));
    }
}
