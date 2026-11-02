use std::time::Duration;

use serde_json::{Value, json};

use super::http::{Api, now};
use super::*;

pub struct GitHub(pub Api);

const THREADS: &str = "query($owner:String!,$name:String!,$number:Int!,$after:String){\
repository(owner:$owner,name:$name){pullRequest(number:$number){\
reviewThreads(first:100,after:$after){pageInfo{hasNextPage endCursor}\
nodes{id isResolved isOutdated path line originalLine diffSide \
comments(first:100){nodes{databaseId body createdAt author{login}}}}}}}}";

const PULLS: &str = "query($owner:String!,$name:String!,$states:[PullRequestState!]){\
repository(owner:$owner,name:$name){pullRequests(states:$states,first:50,\
orderBy:{field:UPDATED_AT,direction:DESC}){nodes{number title state isDraft headRefName baseRefName \
updatedAt url author{login} headRepository{nameWithOwner} assignees(first:100){nodes{login}} \
reviewRequests(first:100){nodes{requestedReviewer{...on User{login}}}} \
latestOpinionatedReviews(first:100){nodes{state author{login}}} \
commits(last:1){nodes{commit{statusCheckRollup{state}}}}}}}}";

fn repo(p: &Project) -> String {
    format!("/repos/{}", p.path)
}

fn status(v: &Value) -> PipelineStatus {
    use PipelineStatus::*;
    match (v["status"].as_str(), v["conclusion"].as_str()) {
        (Some("completed"), Some("success" | "neutral")) => Success,
        (Some("completed"), Some("cancelled")) => Canceled,
        (Some("completed"), Some("skipped" | "stale")) => Skipped,
        (Some("completed"), Some("action_required")) => Manual,
        (Some("completed"), _) => Failed,
        (Some("in_progress"), _) => Running,
        _ => Pending,
    }
}

fn pipeline(p: &Project, v: &Value) -> Pipeline {
    let done = v["status"] == "completed";
    Pipeline {
        id: v["id"].as_u64().unwrap_or_default(),
        project: p.clone(),
        status: status(v),
        ref_: s(&v["head_branch"]),
        sha: s(&v["head_sha"]),
        source: s(&v["event"]),
        created: time(&v["created_at"]),
        started: opt_time(&v["run_started_at"]),
        finished: done.then(|| time(&v["updated_at"])),
        web_url: s(&v["html_url"]),
        cancellable: !done,
        retryable: done,
    }
}

/// A general comment or a review's body: a thread of one note.
fn one(v: &Value, time_key: &str) -> Thread {
    Thread {
        id: s(&v["node_id"]),
        resolvable: false,
        resolved: false,
        replyable: false,
        reply_to: String::new(),
        position: None,
        notes: vec![Note {
            id: s(&v["node_id"]),
            author: s(&v["user"]["login"]),
            body: s(&v["body"]),
            time: time(&v[time_key]),
        }],
    }
}

fn mutation(name: &str, input: &str) -> String {
    format!("mutation($id:ID!){{{name}(input:{{{input}:$id}}){{clientMutationId}}}}")
}

impl GitHub {
    fn runs(&self, p: &Project, q: &str) -> Result<Vec<Pipeline>> {
        let v = self.0.get(&format!("{}/actions/runs?{q}", repo(p)))?;
        Ok(arr(&v["workflow_runs"]).map(|r| pipeline(p, r)).collect())
    }

    fn reviews(&self, p: &Project, n: u64) -> Result<Vec<Value>> {
        let path = format!("{}/pulls/{n}/reviews?per_page=100", repo(p));
        self.0.list(&path, None, usize::MAX)
    }

    /// The PR with its reviews, runs and source repo; `base` is the target repo's JSON.
    fn full(&self, p: &Project, n: u64, base: &Value) -> Result<MergeRequest> {
        use MergeMethod::*;
        let v = self.0.get(&format!("{}/pulls/{n}", repo(p)))?;
        let head_sha = s(&v["head"]["sha"]);
        let runs = self.runs(p, &format!("head_sha={head_sha}&per_page=100"))?;
        // Each reviewer's latest approving or blocking review.
        let mut last: Vec<(String, String)> = vec![];
        for r in self.reviews(p, n)? {
            let (user, st) = (s(&r["user"]["login"]), s(&r["state"]));
            if matches!(st.as_str(), "APPROVED" | "CHANGES_REQUESTED" | "DISMISSED") {
                last.retain(|(u, _)| *u != user);
                last.push((user, st));
            }
        }
        let approved_by: Vec<String> = last
            .iter()
            .filter(|(_, st)| st == "APPROVED")
            .map(|(u, _)| u.clone())
            .collect();
        let mut reviewers: Vec<String> = arr(&v["requested_reviewers"])
            .map(|r| s(&r["login"]))
            .collect();
        for (u, _) in &last {
            if !reviewers.contains(u) {
                reviewers.push(u.clone());
            }
        }
        let src_name = s(&v["head"]["repo"]["full_name"]);
        let src = match src_name.as_str() {
            "" => Value::Null,
            n if n.eq_ignore_ascii_case(&p.path) => base.clone(),
            n => self.0.get(&format!("/repos/{n}"))?,
        };
        let state = match (
            v["state"].as_str(),
            v["merged"] == true || !v["merged_at"].is_null(),
        ) {
            (_, true) => MrState::Merged,
            (Some("open"), _) => MrState::Open,
            _ => MrState::Closed,
        };
        let blocking = match (&state, v["mergeable_state"].as_str()) {
            (MrState::Open, Some("clean" | "unstable" | "has_hooks")) => None,
            (MrState::Open, Some("dirty")) => Some("conflicts"),
            (MrState::Open, Some("blocked")) => Some("blocked by branch protection"),
            (MrState::Open, Some("behind")) => Some("the head branch is behind the base"),
            (MrState::Open, Some("draft")) => Some("draft"),
            (MrState::Open, _) => Some("checking mergeability"),
            _ => Some("not open"),
        };
        let allowed = |k: &str| base[k] != false;
        Ok(MergeRequest {
            provider: self.0.account.provider.clone(),
            number: n,
            title: s(&v["title"]),
            description: s(&v["body"]),
            state,
            draft: v["draft"] == true,
            author: s(&v["user"]["login"]),
            source_project: Project {
                host: p.host.clone(),
                path: if src_name.is_empty() {
                    p.path.clone()
                } else {
                    src_name
                },
            },
            source_branch: s(&v["head"]["ref"]),
            target_project: p.clone(),
            target_branch: s(&v["base"]["ref"]),
            base_sha: s(&v["base"]["sha"]),
            head_sha,
            labels: arr(&v["labels"]).map(|l| s(&l["name"])).collect(),
            reviewers,
            approved: approved_by.contains(&self.0.account.username),
            approved_by,
            blocking: blocking.map(String::from),
            merge_methods: [
                (allowed("allow_merge_commit"), Merge),
                (allowed("allow_squash_merge"), Squash),
                (allowed("allow_rebase_merge"), Rebase),
            ]
            .into_iter()
            .filter_map(|(ok, m)| ok.then_some(m))
            .collect(),
            can_delete_source_branch: src["permissions"]["push"] == true
                && src["default_branch"] != v["head"]["ref"],
            auto_merge: !v["auto_merge"].is_null(),
            pipeline: (!runs.is_empty()).then(|| rollup(runs.iter().map(|r| r.status.clone()))),
            web_url: s(&v["html_url"]),
            project_url: s(&v["base"]["repo"]["html_url"]),
            updated: time(&v["updated_at"]),
        })
    }

    /// Deletes the merged branch in its source project, only with push access there and while
    /// it still points at the merged head.
    fn delete_branch(&self, head: &Value, sha: &str) -> Result<()> {
        let src = s(&head["repo"]["full_name"]);
        if src.is_empty() || self.0.get(&format!("/repos/{src}"))?["permissions"]["push"] != true {
            return Ok(());
        }
        let branch = enc(&s(&head["ref"])).replace("%2F", "/");
        let at = self.0.get(&format!("/repos/{src}/git/ref/heads/{branch}"));
        if at.is_ok_and(|r| r["object"]["sha"] == sha) {
            self.0.send(
                "DELETE",
                &format!("/repos/{src}/git/refs/heads/{branch}"),
                None,
            )?;
        }
        Ok(())
    }
}

impl Forge for GitHub {
    /// One GraphQL request, with only what the list shows; filtered here, as REST would be.
    fn mrs(&self, p: &Project, filter: MrFilter) -> Result<Vec<MergeRequest>> {
        let (owner, name) = p.path.split_once('/').unwrap_or_default();
        let states = match filter {
            MrFilter::All => json!(["OPEN", "CLOSED", "MERGED"]),
            _ => json!(["OPEN"]),
        };
        let vars = json!({"owner": owner, "name": name, "states": states});
        let d = self.0.graphql(PULLS, vars)?;
        let me = self.0.account.username.as_str();
        Ok(arr(&d["repository"]["pullRequests"]["nodes"])
            .filter(|v| match filter {
                MrFilter::Mine => v["author"]["login"] == me,
                MrFilter::ReviewRequested => arr(&v["reviewRequests"]["nodes"])
                    .any(|r| r["requestedReviewer"]["login"] == me),
                MrFilter::Assigned => arr(&v["assignees"]["nodes"]).any(|u| u["login"] == me),
                _ => true,
            })
            .map(|v| {
                let approved_by: Vec<String> = arr(&v["latestOpinionatedReviews"]["nodes"])
                    .filter(|r| r["state"] == "APPROVED")
                    .map(|r| s(&r["author"]["login"]))
                    .collect();
                let rollup = &v["commits"]["nodes"][0]["commit"]["statusCheckRollup"]["state"];
                let source = v["headRepository"]["nameWithOwner"].as_str();
                MergeRequest {
                    provider: self.0.account.provider.clone(),
                    number: v["number"].as_u64().unwrap_or_default(),
                    title: s(&v["title"]),
                    description: String::new(),
                    state: match v["state"].as_str() {
                        Some("OPEN") => MrState::Open,
                        Some("MERGED") => MrState::Merged,
                        _ => MrState::Closed,
                    },
                    draft: v["isDraft"] == true,
                    author: s(&v["author"]["login"]),
                    source_project: Project {
                        host: p.host.clone(),
                        path: source.unwrap_or(&p.path).into(),
                    },
                    source_branch: s(&v["headRefName"]),
                    target_project: p.clone(),
                    target_branch: s(&v["baseRefName"]),
                    base_sha: String::new(),
                    head_sha: String::new(),
                    labels: vec![],
                    reviewers: vec![],
                    approved: approved_by.iter().any(|u| u == me),
                    approved_by,
                    blocking: None,
                    merge_methods: vec![],
                    can_delete_source_branch: false,
                    auto_merge: false,
                    pipeline: rollup.as_str().map(|r| match r {
                        "SUCCESS" => PipelineStatus::Success,
                        "FAILURE" | "ERROR" => PipelineStatus::Failed,
                        _ => PipelineStatus::Pending,
                    }),
                    web_url: s(&v["url"]),
                    project_url: String::new(),
                    updated: time(&v["updatedAt"]),
                }
            })
            .collect())
    }

    fn find_mr(&self, p: &Project, source: &Project, branch: &str) -> Result<Option<MergeRequest>> {
        let owner = source.path.split('/').next().unwrap_or_default();
        let head = enc(&format!("{owner}:{branch}"));
        let path = format!("{}/pulls?state=open&head={head}&per_page=100", repo(p));
        let list = self.0.list(&path, None, 100)?;
        let found = list.iter().find(|v| {
            v["head"]["ref"] == branch
                && v["head"]["repo"]["full_name"]
                    .as_str()
                    .is_some_and(|n| n.eq_ignore_ascii_case(&source.path))
        });
        match found.and_then(|v| v["number"].as_u64()) {
            Some(n) => Ok(Some(self.mr(p, n)?)),
            None => Ok(None),
        }
    }

    fn mr(&self, p: &Project, n: u64) -> Result<MergeRequest> {
        self.full(p, n, &self.0.get(&repo(p))?)
    }

    fn threads(&self, p: &Project, n: u64) -> Result<Vec<Thread>> {
        let (owner, name) = p.path.split_once('/').unwrap_or_default();
        let mut out = vec![];
        let mut after = Value::Null;
        loop {
            let vars = json!({"owner": owner, "name": name, "number": n, "after": after});
            let d = self.0.graphql(THREADS, vars)?;
            let t = &d["repository"]["pullRequest"]["reviewThreads"];
            for x in arr(&t["nodes"]) {
                let notes: Vec<Note> = arr(&x["comments"]["nodes"])
                    .map(|c| Note {
                        id: c["databaseId"].to_string(),
                        author: s(&c["author"]["login"]),
                        body: s(&c["body"]),
                        time: time(&c["createdAt"]),
                    })
                    .collect();
                let line = x["line"].as_u64().or(x["originalLine"].as_u64());
                out.push(Thread {
                    id: s(&x["id"]),
                    resolvable: true,
                    resolved: x["isResolved"] == true,
                    replyable: true,
                    reply_to: notes.first().map(|n| n.id.clone()).unwrap_or_default(),
                    position: line.map(|l| Position {
                        path: s(&x["path"]),
                        side: if x["diffSide"] == "LEFT" {
                            DiffSide::Old
                        } else {
                            DiffSide::New
                        },
                        line: l as u32,
                        outdated: x["isOutdated"] == true,
                    }),
                    notes,
                });
            }
            if t["pageInfo"]["hasNextPage"] != true {
                break;
            }
            after = t["pageInfo"]["endCursor"].clone();
        }
        let comments = format!("{}/issues/{n}/comments?per_page=100", repo(p));
        out.extend(
            self.0
                .list(&comments, None, usize::MAX)?
                .iter()
                .map(|c| one(c, "created_at")),
        );
        let reviews = self.reviews(p, n)?;
        out.extend(
            reviews
                .iter()
                .filter(|r| r["state"] != "PENDING" && !s(&r["body"]).is_empty())
                .map(|r| one(r, "submitted_at")),
        );
        out.sort_by_key(|t| t.notes.first().map(|n| n.time));
        Ok(out)
    }

    fn act(&self, p: &Project, n: u64, a: &MrAction) -> Result<()> {
        let pr = format!("{}/pulls/{n}", repo(p));
        let post = |path: String, body: Value| self.0.send("POST", &path, Some(body)).map(drop);
        let node = || -> Result<Value> { Ok(self.0.get(&pr)?["node_id"].clone()) };
        let patch = |state: &str| {
            let body = json!({"state": state});
            self.0.send("PATCH", &pr, Some(body)).map(drop)
        };
        match a {
            MrAction::Comment { body } => post(
                format!("{}/issues/{n}/comments", repo(p)),
                json!({"body": body}),
            ),
            MrAction::Reply { reply_to, body } => post(
                format!("{pr}/comments/{}/replies", enc(reply_to)),
                json!({"body": body}),
            ),
            MrAction::LineComment {
                body,
                path,
                old_line,
                new_line,
                head_sha,
                ..
            } => {
                let (line, side) = match new_line {
                    Some(l) => (Some(l), "RIGHT"),
                    None => (old_line.as_ref(), "LEFT"),
                };
                let b = json!({"body": body, "commit_id": head_sha, "path": path, "line": line, "side": side});
                post(format!("{pr}/comments"), b)
            }
            MrAction::Resolve {
                thread_id,
                resolved,
            } => {
                let m = match resolved {
                    true => "resolveReviewThread",
                    false => "unresolveReviewThread",
                };
                self.0
                    .graphql(&mutation(m, "threadId"), json!({"id": thread_id}))
                    .map(drop)
            }
            MrAction::Approve => post(format!("{pr}/reviews"), json!({"event": "APPROVE"})),
            MrAction::RequestChanges { body } => post(
                format!("{pr}/reviews"),
                json!({"event": "REQUEST_CHANGES", "body": body}),
            ),
            MrAction::Unapprove => Err(unsupported("Unapprove")),
            MrAction::Merge {
                method,
                delete_source_branch,
                auto_merge,
                head_sha,
            } => {
                let m = match method {
                    MergeMethod::Merge => "merge",
                    MergeMethod::Squash => "squash",
                    MergeMethod::Rebase => "rebase",
                };
                if *auto_merge {
                    if *delete_source_branch {
                        return Err(Error::Other(
                            "GitHub cannot delete the branch after an auto-merge".into(),
                        ));
                    }
                    let q = "mutation($id:ID!,$m:PullRequestMergeMethod!,$sha:GitObjectID){\
                        enablePullRequestAutoMerge(input:{pullRequestId:$id,mergeMethod:$m,\
                        expectedHeadOid:$sha}){clientMutationId}}";
                    let vars = json!({"id": node()?, "m": m.to_uppercase(), "sha": head_sha});
                    return self.0.graphql(q, vars).map(drop);
                }
                let head = self.0.get(&pr)?["head"].clone();
                let body = json!({"merge_method": m, "sha": head_sha});
                self.0.send("PUT", &format!("{pr}/merge"), Some(body))?;
                if *delete_source_branch {
                    self.delete_branch(&head, head_sha)?;
                }
                Ok(())
            }
            MrAction::SetDraft { draft } => {
                let m = match draft {
                    true => "convertPullRequestToDraft",
                    false => "markPullRequestReadyForReview",
                };
                self.0
                    .graphql(&mutation(m, "pullRequestId"), json!({"id": node()?}))
                    .map(drop)
            }
            MrAction::Close => patch("closed"),
            MrAction::Reopen => patch("open"),
        }
    }

    fn default_branch(&self, p: &Project) -> Result<String> {
        Ok(s(&self.0.get(&repo(p))?["default_branch"]))
    }

    fn create(
        &self,
        p: &Project,
        source: &Project,
        branch: &str,
        input: &CreateMr,
    ) -> Result<MergeRequest> {
        let head = match source == p {
            true => branch.to_string(),
            false => format!(
                "{}:{branch}",
                source.path.split('/').next().unwrap_or_default()
            ),
        };
        let body = json!({
            "title": input.title, "body": input.description, "head": head,
            "base": input.target_branch, "draft": input.draft,
        });
        let v = self
            .0
            .send("POST", &format!("{}/pulls", repo(p)), Some(body))?;
        self.mr(p, v["number"].as_u64().unwrap_or_default())
    }

    fn pipelines(
        &self,
        p: &Project,
        branch: Option<(&str, Option<&str>)>,
    ) -> Result<Vec<Pipeline>> {
        let q = match branch {
            Some((b, sha)) => format!(
                "&branch={}{}",
                enc(b),
                sha.map(|s| format!("&head_sha={s}")).unwrap_or_default()
            ),
            None => String::new(),
        };
        self.runs(p, &format!("per_page=20{q}"))
    }

    fn mr_pipelines(&self, p: &Project, n: u64) -> Result<Vec<Pipeline>> {
        let sha = s(&self.0.get(&format!("{}/pulls/{n}", repo(p)))?["head"]["sha"]);
        self.runs(p, &format!("head_sha={sha}&per_page=100"))
    }

    fn jobs(&self, p: &Project, id: u64) -> Result<Vec<Stage>> {
        let path = format!("{}/actions/runs/{id}/jobs?per_page=100", repo(p));
        let list = self.0.list(&path, Some("jobs"), usize::MAX)?;
        let Some(first) = list.first() else {
            return Ok(vec![]);
        };
        let jobs: Vec<Job> = list
            .iter()
            .map(|j| {
                let done = j["status"] == "completed";
                let took = opt_time(&j["completed_at"]).zip(opt_time(&j["started_at"]));
                Job {
                    id: j["id"].as_u64().unwrap_or_default(),
                    name: s(&j["name"]),
                    status: status(j),
                    duration: took.filter(|_| done).map(|(e, b)| (e - b).max(0) as u64),
                    allow_failure: false,
                    cancellable: false,
                    playable: false,
                    retryable: done,
                    live_log: false,
                    web_url: s(&j["html_url"]),
                    steps: arr(&j["steps"])
                        .map(|st| Step {
                            name: s(&st["name"]),
                            status: status(st),
                        })
                        .collect(),
                }
            })
            .collect();
        Ok(vec![Stage {
            name: s(&first["workflow_name"]),
            status: rollup(jobs.iter().map(|j| j.status.clone())),
            jobs,
        }])
    }

    fn pipeline_action(&self, p: &Project, id: u64, a: PipelineAction) -> Result<()> {
        let verb = match a {
            PipelineAction::Retry => "rerun",
            PipelineAction::RetryFailed => "rerun-failed-jobs",
            PipelineAction::Cancel => "cancel",
        };
        let path = format!("{}/actions/runs/{id}/{verb}", repo(p));
        self.0.send("POST", &path, None).map(drop)
    }

    fn job_action(&self, p: &Project, id: u64, a: JobAction) -> Result<()> {
        match a {
            JobAction::Retry => {
                let path = format!("{}/actions/jobs/{id}/rerun", repo(p));
                self.0.send("POST", &path, None).map(drop)
            }
            JobAction::Cancel => Err(unsupported("Cancelling one job")),
            JobAction::Play => Err(unsupported("Playing a job")),
        }
    }

    /// GitHub serves a job's log only once it has finished.
    fn job_log(&self, p: &Project, id: u64, offset: u64) -> Result<JobLogChunk> {
        let job = self.0.get(&format!("{}/actions/jobs/{id}", repo(p)))?;
        if job["status"] != "completed" {
            return Ok(chunk(&[], offset, false));
        }
        let log = self.0.log(&format!("{}/actions/jobs/{id}/logs", repo(p)))?;
        Ok(chunk(&log, offset, true))
    }

    fn workflows(&self, p: &Project) -> Result<Vec<Workflow>> {
        let path = format!("{}/actions/workflows?per_page=100", repo(p));
        let list = self.0.list(&path, Some("workflows"), usize::MAX)?;
        Ok(list
            .iter()
            .filter(|w| w["state"] == "active")
            .map(|w| Workflow {
                id: w["id"].to_string(),
                name: s(&w["name"]),
            })
            .collect())
    }

    /// Dispatches the workflow and waits up to 10 s for its run to appear.
    fn run(&self, input: &RunPipeline) -> Result<Pipeline> {
        let p = &input.project;
        let wf = input.workflow.as_deref();
        let wf = enc(wf.ok_or_else(|| Error::Other("No workflow chosen".into()))?);
        let runs = format!(
            "{}/actions/workflows/{wf}/runs?event=workflow_dispatch&branch={}&per_page=1",
            repo(p),
            enc(&input.branch)
        );
        let latest = || -> Result<Value> { Ok(self.0.get(&runs)?["workflow_runs"][0].clone()) };
        let before = latest()?["id"].as_u64().unwrap_or_default();
        let inputs: serde_json::Map<String, Value> = input
            .variables
            .iter()
            .map(|(k, v)| (k.clone(), json!(v)))
            .collect();
        let body = json!({"ref": input.branch, "inputs": inputs});
        self.0.send(
            "POST",
            &format!("{}/actions/workflows/{wf}/dispatches", repo(p)),
            Some(body),
        )?;
        let start = now();
        while now() - start < 10 {
            let r = latest()?;
            if r["id"].as_u64().unwrap_or_default() > before {
                return Ok(pipeline(p, &r));
            }
            std::thread::sleep(Duration::from_secs(1));
        }
        Err(Error::Other(
            "The workflow was dispatched; its run has not appeared yet".into(),
        ))
    }
}
