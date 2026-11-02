use std::collections::HashMap;
use std::sync::Mutex;

use serde_json::{Value, json};

use super::http::Api;
use super::*;

pub struct GitLab {
    api: Api,
    /// Project paths by id, for forks and MR pipelines.
    paths: Mutex<HashMap<u64, String>>,
}

const ACTIVE: [&str; 6] = [
    "created",
    "waiting_for_resource",
    "preparing",
    "pending",
    "running",
    "scheduled",
];

fn pr(p: &Project) -> String {
    format!("/projects/{}", enc(&p.path))
}

fn status(s: &str) -> PipelineStatus {
    use PipelineStatus::*;
    match s {
        "running" | "canceling" => Running,
        "success" => Success,
        "failed" => Failed,
        "canceled" => Canceled,
        "skipped" => Skipped,
        "manual" => Manual,
        _ => Pending,
    }
}

fn mr_state(s: &str) -> MrState {
    match s {
        "opened" => MrState::Open,
        "merged" => MrState::Merged,
        _ => MrState::Closed,
    }
}

fn draft_title(t: &str, draft: bool) -> String {
    let lower = t.to_ascii_lowercase();
    let bare = ["draft:", "[draft]", "(draft)"]
        .iter()
        .find(|d| lower.starts_with(*d))
        .map_or(t, |d| t[d.len()..].trim_start());
    if draft {
        format!("Draft: {bare}")
    } else {
        bare.into()
    }
}

fn pipeline(p: &Project, v: &Value) -> Pipeline {
    let st = s(&v["status"]);
    Pipeline {
        id: v["id"].as_u64().unwrap_or_default(),
        project: p.clone(),
        status: status(&st),
        ref_: s(&v["ref"]),
        sha: s(&v["sha"]),
        source: s(&v["source"]),
        created: time(&v["created_at"]),
        started: opt_time(&v["started_at"]),
        finished: opt_time(&v["finished_at"]),
        web_url: s(&v["web_url"]),
        cancellable: ACTIVE.contains(&st.as_str()),
        retryable: matches!(st.as_str(), "failed" | "canceled"),
    }
}

impl GitLab {
    pub fn new(api: Api) -> GitLab {
        GitLab {
            api,
            paths: Mutex::default(),
        }
    }

    fn project_of(&self, p: &Project, id: &Value) -> Result<Project> {
        let id = id.as_u64().unwrap_or_default();
        let known = self.paths.lock().unwrap().get(&id).cloned();
        let path = match known {
            Some(path) => path,
            None => {
                let path = s(&self.api.get(&format!("/projects/{id}"))?["path_with_namespace"]);
                self.paths.lock().unwrap().insert(id, path.clone());
                path
            }
        };
        Ok(Project {
            host: p.host.clone(),
            path,
        })
    }

    fn mr_json(&self, p: &Project, n: u64) -> Result<Value> {
        self.api.get(&format!("{}/merge_requests/{n}", pr(p)))
    }

    /// `project` is the target project's JSON.
    fn full(&self, p: &Project, v: &Value, project: &Value) -> Result<MergeRequest> {
        use MergeMethod::*;
        let n = v["iid"].as_u64().unwrap_or_default();
        let approvals = self
            .api
            .get(&format!("{}/merge_requests/{n}/approvals", pr(p)))?;
        let source_project = match v["source_project_id"] == v["target_project_id"] {
            true => p.clone(),
            false => self.project_of(p, &v["source_project_id"])?,
        };
        let approved_by: Vec<String> = arr(&approvals["approved_by"])
            .map(|a| s(&a["user"]["username"]))
            .collect();
        let blocking = match v["detailed_merge_status"].as_str() {
            Some("mergeable") => None,
            Some(d) => Some(d.replace('_', " ")),
            None => (v["merge_status"] != "can_be_merged").then(|| "cannot be merged".into()),
        };
        Ok(MergeRequest {
            provider: self.api.account.provider.clone(),
            number: n,
            title: s(&v["title"]),
            description: s(&v["description"]),
            state: mr_state(v["state"].as_str().unwrap_or_default()),
            draft: v["draft"] == true || v["work_in_progress"] == true,
            author: s(&v["author"]["username"]),
            source_project,
            source_branch: s(&v["source_branch"]),
            target_project: p.clone(),
            target_branch: s(&v["target_branch"]),
            base_sha: s(&v["diff_refs"]["base_sha"]),
            head_sha: s(&v["diff_refs"]["head_sha"]),
            labels: arr(&v["labels"]).map(s).collect(),
            reviewers: arr(&v["reviewers"]).map(|r| s(&r["username"])).collect(),
            approved: approvals["user_has_approved"] == true
                || approved_by.contains(&self.api.account.username),
            approved_by,
            blocking,
            merge_methods: match project["squash_option"].as_str() {
                Some("always") => vec![Squash],
                Some("never") => vec![Merge],
                _ => vec![Merge, Squash],
            },
            can_delete_source_branch: true,
            auto_merge: v["merge_when_pipeline_succeeds"] == true,
            pipeline: v["head_pipeline"]["status"].as_str().map(status),
            web_url: s(&v["web_url"]),
            project_url: s(&project["web_url"]),
            updated: time(&v["updated_at"]),
        })
    }

    /// The list in one GraphQL request, with only what the list shows.
    fn listed(&self, p: &Project, filter: &MrFilter) -> Result<Vec<MergeRequest>> {
        let me = json!(self.api.account.username);
        let args = match filter {
            MrFilter::Open => ",state:opened".into(),
            MrFilter::Mine => format!(",state:opened,authorUsername:{me}"),
            MrFilter::ReviewRequested => format!(",state:opened,reviewerUsername:{me}"),
            MrFilter::Assigned => format!(",state:opened,assigneeUsername:{me}"),
            MrFilter::All => String::new(),
        };
        let q = format!(
            "query($path:ID!){{project(fullPath:$path){{mergeRequests(sort:UPDATED_DESC,first:{LIST}{args})\
             {{nodes{{iid title state draft sourceBranch targetBranch updatedAt webUrl author{{username}} \
             approvedBy{{nodes{{username}}}} headPipeline{{status}} sourceProject{{fullPath}}}}}}}}}}"
        );
        let d = self.api.graphql(&q, json!({"path": p.path}))?;
        let nodes = d["project"]["mergeRequests"]["nodes"].as_array();
        let nodes = nodes.ok_or_else(|| Error::Other(format!("No project {}", p.path)))?;
        Ok(nodes
            .iter()
            .map(|v| {
                let approved_by: Vec<String> = arr(&v["approvedBy"]["nodes"])
                    .map(|u| s(&u["username"]))
                    .collect();
                let source = v["sourceProject"]["fullPath"].as_str();
                MergeRequest {
                    provider: self.api.account.provider.clone(),
                    number: s(&v["iid"]).parse().unwrap_or_default(),
                    title: s(&v["title"]),
                    description: String::new(),
                    state: mr_state(&s(&v["state"])),
                    draft: v["draft"] == true,
                    author: s(&v["author"]["username"]),
                    source_project: Project {
                        host: p.host.clone(),
                        path: source.unwrap_or(&p.path).into(),
                    },
                    source_branch: s(&v["sourceBranch"]),
                    target_project: p.clone(),
                    target_branch: s(&v["targetBranch"]),
                    base_sha: String::new(),
                    head_sha: String::new(),
                    labels: vec![],
                    reviewers: vec![],
                    approved: approved_by.contains(&self.api.account.username),
                    approved_by,
                    blocking: None,
                    merge_methods: vec![],
                    can_delete_source_branch: false,
                    auto_merge: false,
                    pipeline: v["headPipeline"]["status"]
                        .as_str()
                        .map(|x| status(&x.to_ascii_lowercase())),
                    web_url: s(&v["webUrl"]),
                    project_url: String::new(),
                    updated: time(&v["updatedAt"]),
                }
            })
            .collect())
    }

    /// Two requests per MR, for instances whose GraphQL lacks a field.
    fn mrs_rest(&self, p: &Project, filter: MrFilter) -> Result<Vec<MergeRequest>> {
        let q = match filter {
            MrFilter::Open => "state=opened".into(),
            MrFilter::Mine => "state=opened&scope=created_by_me".into(),
            MrFilter::ReviewRequested => format!(
                "state=opened&reviewer_username={}",
                enc(&self.api.account.username)
            ),
            MrFilter::Assigned => "state=opened&scope=assigned_to_me".into(),
            MrFilter::All => "state=all".to_string(),
        };
        let path = format!(
            "{}/merge_requests?{q}&order_by=updated_at&per_page={LIST}",
            pr(p)
        );
        let list = self.api.list(&path, None, LIST)?;
        let project = self.api.get(&pr(p))?;
        each(&list, |m| {
            let v = self.mr_json(p, m["iid"].as_u64().unwrap_or_default())?;
            self.full(p, &v, &project)
        })
    }

    /// `auto_merge` from GitLab 17.11, `merge_when_pipeline_succeeds` before.
    fn auto_merge_key(&self) -> Result<&'static str> {
        let v = s(&self.api.get("/version")?["version"]);
        let n: Vec<u32> = v
            .split(['.', '-'])
            .take(2)
            .filter_map(|x| x.parse().ok())
            .collect();
        Ok(match n >= vec![17, 11] {
            true => "auto_merge",
            false => "merge_when_pipeline_succeeds",
        })
    }
}

impl Forge for GitLab {
    fn mrs(&self, p: &Project, filter: MrFilter) -> Result<Vec<MergeRequest>> {
        self.listed(p, &filter)
            .or_else(|_| self.mrs_rest(p, filter))
    }

    fn find_mr(&self, p: &Project, source: &Project, branch: &str) -> Result<Option<MergeRequest>> {
        let id = self.api.get(&pr(source))?["id"].clone();
        let path = format!(
            "{}/merge_requests?state=opened&source_branch={}&per_page=100",
            pr(p),
            enc(branch)
        );
        let list = self.api.list(&path, None, 100)?;
        match list.iter().find(|m| m["source_project_id"] == id) {
            Some(m) => Ok(Some(self.mr(p, m["iid"].as_u64().unwrap_or_default())?)),
            None => Ok(None),
        }
    }

    fn mr(&self, p: &Project, n: u64) -> Result<MergeRequest> {
        self.full(p, &self.mr_json(p, n)?, &self.api.get(&pr(p))?)
    }

    fn threads(&self, p: &Project, n: u64) -> Result<Vec<Thread>> {
        let head = self.mr_json(p, n)?["diff_refs"]["head_sha"].clone();
        let path = format!("{}/merge_requests/{n}/discussions?per_page=100", pr(p));
        let ds = self.api.list(&path, None, usize::MAX)?;
        Ok(ds
            .iter()
            .filter_map(|d| {
                let notes: Vec<&Value> = arr(&d["notes"]).filter(|n| n["system"] != true).collect();
                let pos = &notes.first()?["position"];
                let new = pos["new_line"].as_u64();
                let (key, side) = match new {
                    Some(_) => ("new_path", DiffSide::New),
                    None => ("old_path", DiffSide::Old),
                };
                let position = pos.is_object().then(|| Position {
                    path: s(&pos[key]),
                    side,
                    line: new.or(pos["old_line"].as_u64()).unwrap_or_default() as u32,
                    outdated: pos["head_sha"] != head,
                });
                let resolvable: Vec<_> = notes.iter().filter(|n| n["resolvable"] == true).collect();
                Some(Thread {
                    id: s(&d["id"]),
                    resolved: !resolvable.is_empty()
                        && resolvable.iter().all(|n| n["resolved"] == true),
                    resolvable: !resolvable.is_empty(),
                    replyable: true,
                    reply_to: s(&d["id"]),
                    position,
                    notes: notes
                        .iter()
                        .map(|n| Note {
                            id: n["id"].to_string(),
                            author: s(&n["author"]["username"]),
                            body: s(&n["body"]),
                            time: time(&n["created_at"]),
                        })
                        .collect(),
                })
            })
            .collect())
    }

    fn act(&self, p: &Project, n: u64, a: &MrAction) -> Result<()> {
        let mr = format!("{}/merge_requests/{n}", pr(p));
        let (method, path, body) = match a {
            MrAction::Comment { body } => ("POST", format!("{mr}/notes"), json!({"body": body})),
            MrAction::Reply { reply_to, body } => (
                "POST",
                format!("{mr}/discussions/{}/notes", enc(reply_to)),
                json!({"body": body}),
            ),
            MrAction::LineComment {
                body,
                path,
                old_path,
                old_line,
                new_line,
                head_sha,
            } => {
                let refs = self.mr_json(p, n)?["diff_refs"].clone();
                let mut pos = json!({
                    "position_type": "text", "base_sha": refs["base_sha"],
                    "start_sha": refs["start_sha"], "head_sha": head_sha,
                    "old_path": old_path, "new_path": path,
                });
                if let Some(l) = old_line {
                    pos["old_line"] = json!(l);
                }
                if let Some(l) = new_line {
                    pos["new_line"] = json!(l);
                }
                let body = json!({"body": body, "position": pos});
                ("POST", format!("{mr}/discussions"), body)
            }
            MrAction::Resolve {
                thread_id,
                resolved,
            } => (
                "PUT",
                format!("{mr}/discussions/{}", enc(thread_id)),
                json!({"resolved": resolved}),
            ),
            MrAction::Approve => ("POST", format!("{mr}/approve"), json!({})),
            MrAction::Unapprove => ("POST", format!("{mr}/unapprove"), json!({})),
            MrAction::RequestChanges { .. } => return Err(unsupported("Request changes")),
            MrAction::Merge {
                method,
                delete_source_branch,
                auto_merge,
                head_sha,
            } => {
                let mut b = json!({
                    "squash": *method == MergeMethod::Squash,
                    "should_remove_source_branch": delete_source_branch, "sha": head_sha,
                });
                if *auto_merge {
                    b[self.auto_merge_key()?] = json!(true);
                }
                ("PUT", format!("{mr}/merge"), b)
            }
            MrAction::SetDraft { draft } => {
                let title = draft_title(&s(&self.mr_json(p, n)?["title"]), *draft);
                ("PUT", mr, json!({"title": title}))
            }
            MrAction::Close => ("PUT", mr, json!({"state_event": "close"})),
            MrAction::Reopen => ("PUT", mr, json!({"state_event": "reopen"})),
        };
        self.api.send(method, &path, Some(body)).map(drop)
    }

    fn default_branch(&self, p: &Project) -> Result<String> {
        Ok(s(&self.api.get(&pr(p))?["default_branch"]))
    }

    fn create(
        &self,
        p: &Project,
        source: &Project,
        branch: &str,
        input: &CreateMr,
    ) -> Result<MergeRequest> {
        let mut b = json!({
            "source_branch": branch, "target_branch": input.target_branch,
            "title": draft_title(&input.title, input.draft), "description": input.description,
        });
        if source != p {
            b["target_project_id"] = self.api.get(&pr(p))?["id"].clone();
        }
        let v = self
            .api
            .send("POST", &format!("{}/merge_requests", pr(source)), Some(b))?;
        self.mr(p, v["iid"].as_u64().unwrap_or_default())
    }

    fn pipelines(
        &self,
        p: &Project,
        branch: Option<(&str, Option<&str>)>,
    ) -> Result<Vec<Pipeline>> {
        let q = match branch {
            Some((b, sha)) => format!(
                "&ref={}{}",
                enc(b),
                sha.map(|s| format!("&sha={s}")).unwrap_or_default()
            ),
            None => String::new(),
        };
        let v = self
            .api
            .get(&format!("{}/pipelines?per_page=20{q}", pr(p)))?;
        Ok(arr(&v).map(|x| pipeline(p, x)).collect())
    }

    fn mr_pipelines(&self, p: &Project, n: u64) -> Result<Vec<Pipeline>> {
        let v = self
            .api
            .get(&format!("{}/merge_requests/{n}/pipelines", pr(p)))?;
        arr(&v)
            .map(|x| Ok(pipeline(&self.project_of(p, &x["project_id"])?, x)))
            .collect()
    }

    fn jobs(&self, p: &Project, id: u64) -> Result<Vec<Stage>> {
        let path = format!("{}/pipelines/{id}/jobs?per_page=100", pr(p));
        let mut jobs = self.api.list(&path, None, usize::MAX)?;
        // Jobs are created stage by stage, so ids give the stage order.
        jobs.sort_by_key(|j| j["id"].as_u64());
        let mut stages: Vec<Stage> = vec![];
        for j in &jobs {
            let st = s(&j["status"]);
            let job = Job {
                id: j["id"].as_u64().unwrap_or_default(),
                name: s(&j["name"]),
                status: status(&st),
                duration: j["duration"].as_f64().map(|d| d as u64),
                allow_failure: j["allow_failure"] == true,
                cancellable: ACTIVE.contains(&st.as_str()),
                playable: st == "manual",
                retryable: matches!(st.as_str(), "failed" | "canceled" | "success"),
                live_log: true,
                web_url: s(&j["web_url"]),
                steps: vec![],
            };
            let name = s(&j["stage"]);
            match stages.iter_mut().find(|s| s.name == name) {
                Some(s) => s.jobs.push(job),
                None => stages.push(Stage {
                    name,
                    status: PipelineStatus::Skipped,
                    jobs: vec![job],
                }),
            }
        }
        for st in &mut stages {
            st.status = rollup(st.jobs.iter().map(|j| {
                match j.allow_failure && j.status == PipelineStatus::Failed {
                    true => PipelineStatus::Success,
                    false => j.status.clone(),
                }
            }));
        }
        Ok(stages)
    }

    fn pipeline_action(&self, p: &Project, id: u64, a: PipelineAction) -> Result<()> {
        let verb = match a {
            PipelineAction::Retry | PipelineAction::RetryFailed => "retry",
            PipelineAction::Cancel => "cancel",
        };
        let path = format!("{}/pipelines/{id}/{verb}", pr(p));
        self.api.send("POST", &path, None).map(drop)
    }

    fn job_action(&self, p: &Project, id: u64, a: JobAction) -> Result<()> {
        let verb = match a {
            JobAction::Retry => "retry",
            JobAction::Cancel => "cancel",
            JobAction::Play => "play",
        };
        self.api
            .send("POST", &format!("{}/jobs/{id}/{verb}", pr(p)), None)
            .map(drop)
    }

    fn job_log(&self, p: &Project, id: u64, offset: u64) -> Result<JobLogChunk> {
        let st = s(&self.api.get(&format!("{}/jobs/{id}", pr(p)))?["status"]);
        let log = self.api.log(&format!("{}/jobs/{id}/trace", pr(p)))?;
        Ok(chunk(
            &log,
            offset,
            !ACTIVE.contains(&st.as_str()) && st != "canceling",
        ))
    }

    fn workflows(&self, _p: &Project) -> Result<Vec<Workflow>> {
        Ok(vec![])
    }

    fn run(&self, input: &RunPipeline) -> Result<Pipeline> {
        let variables: Vec<Value> = input
            .variables
            .iter()
            .map(|(k, v)| json!({"key": k, "value": v}))
            .collect();
        let body = json!({"ref": input.branch, "variables": variables});
        let v = self.api.send(
            "POST",
            &format!("{}/pipeline", pr(&input.project)),
            Some(body),
        )?;
        Ok(pipeline(&input.project, &v))
    }
}
