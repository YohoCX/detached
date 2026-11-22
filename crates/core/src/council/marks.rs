//! Trust marks for councils, personal and per machine in settings.json, and what they make of a
//! subject: trusted runs in orchestrator mode, anything else gets the read-only sandbox.

use std::{collections::BTreeMap, path::Path};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::record::{Of, Subject};
use crate::env::Env;
use crate::error::Result;
use crate::git::runner::{self, Kind, Opts, out};
use crate::state::AppState;

/// The settings block.
pub const BLOCK: &str = "council";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Council {
    /// Workspaces marked trusted, by name.
    pub workspaces: Vec<Mark>,
    /// Repos marked trusted, by path.
    pub repos: Vec<Mark>,
    pub branches: Vec<Branch>,
    /// The owner's emails besides each repo's `user.email`.
    pub identities: Vec<String>,
    /// Live participants at most, chat turns counted first.
    pub cap: usize,
    /// Seconds without an update that make a participant stalled, and a tool call's own limit.
    pub stall: u64,
    pub tool: u64,
    #[serde(flatten)]
    pub other: BTreeMap<String, Value>,
}

impl Default for Council {
    fn default() -> Self {
        Council {
            workspaces: vec![],
            repos: vec![],
            branches: vec![],
            identities: vec![],
            cap: 4,
            stall: 120,
            tool: 1200,
            other: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Mark {
    pub name: String,
    /// The repo's own remote; `origin` when None.
    pub remote: Option<String>,
    /// MRs from forks count as trusted too.
    pub forks: bool,
    #[serde(flatten)]
    pub other: BTreeMap<String, Value>,
}

/// A local branch the owner marked trusted at `head`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Branch {
    pub repo: String,
    pub branch: String,
    pub head: String,
    #[serde(flatten)]
    pub other: BTreeMap<String, Value>,
}

pub fn load(core: &AppState) -> Council {
    core.get_settings().block(BLOCK).unwrap_or_default()
}

/// Changes the marks and limits in settings.json.
pub fn change(core: &AppState, f: impl FnOnce(&mut Council)) -> Result<()> {
    core.change_block(BLOCK, f).map(drop)
}

/// Marks `repo` trusted as a whole unless a repo or workspace mark already covers it, whose remote
/// and forks stay as the owner set them.
pub fn mark_repo(core: &AppState, repo: &str) -> Result<()> {
    if whole(core, &load(core), repo).is_some() {
        return Ok(());
    }
    change(core, |p| {
        p.repos.push(Mark {
            name: repo.into(),
            ..Mark::default()
        })
    })
}

/// A repo or workspace mark trusts `repo` as a whole.
pub fn covers(core: &AppState, repo: &str) -> bool {
    whole(core, &load(core), repo).is_some()
}

/// What decides a subject's run.
#[derive(Clone, Debug, PartialEq)]
pub struct Trust {
    pub trusted: bool,
    /// "trusted by workspace Work", "its upstream is refs/pull/7/head".
    pub why: String,
}

fn yes(why: String) -> Result<Trust> {
    Ok(Trust { trusted: true, why })
}

fn no(why: String) -> Result<Trust> {
    Ok(Trust {
        trusted: false,
        why,
    })
}

/// The mark that trusts `repo` as a whole, with its words.
fn whole<'a>(core: &AppState, p: &'a Council, repo: &str) -> Option<(&'a Mark, String)> {
    if let Some(m) = p.repos.iter().find(|m| m.name == repo) {
        return Some((m, "trusted by repo".into()));
    }
    let settings = core.get_settings();
    let ws = settings.workspaces.iter().filter(|w| {
        let inside = w.folders.iter().any(|f| Path::new(repo).starts_with(f));
        inside && !w.excluded_repos.iter().any(|r| r == repo)
    });
    ws.filter_map(|w| p.workspaces.iter().find(|m| m.name == w.name))
        .map(|m| (m, format!("trusted by workspace {}", m.name)))
        .next()
}

pub fn classify(core: &AppState, p: &Council, s: &Subject) -> Result<Trust> {
    let env = core.env();
    let repo = Path::new(&s.repo);
    if let Of::Mr { remote, fork, .. } = &s.of {
        let Some((m, why)) = whole(core, p, &s.repo) else {
            return no("an MR of a repo not marked trusted".into());
        };
        let own = m.remote.as_deref().unwrap_or("origin");
        return match (remote == own, *fork && !m.forks) {
            (false, _) => no(format!("an MR on {remote}, not on {own}")),
            (_, true) => no("an MR from a fork, and forks are not included".into()),
            _ => yes(why),
        };
    }
    let mark = |why: String| match branch_mark(&env, p, s) {
        Some(Ok(())) => yes("marked trusted by you".into()),
        Some(Err(dropped)) => no(format!("your mark dropped: {dropped}")),
        None => no(why),
    };
    let Some((m, why)) = whole(core, p, &s.repo) else {
        return match s.remote {
            true => no("a remote branch of a repo not marked trusted".into()),
            false => mark("not marked trusted".into()),
        };
    };
    let own = m.remote.as_deref().unwrap_or("origin");
    if s.remote {
        let (remote, branch) = split_remote(&env, repo, &s.branch)?;
        if remote != own {
            return no(format!("a branch of {remote}, not of {own}"));
        }
        return match from_heads(&env, repo, &remote, &branch)? {
            true => yes(why),
            false => no(format!(
                "{own}'s fetch refspecs map something other than branches to it"
            )),
        };
    }
    match upstream(&env, repo, &s.branch)? {
        None => yes(format!("{why}, your local branch")),
        Some((r, merge)) if r == own && merge.starts_with("refs/heads/") => yes(why),
        Some((r, merge)) => mark(format!("its upstream is {merge} on {r}")),
    }
}

/// The marked branch's check: None when unmarked, else whether the mark holds and why not.
fn branch_mark(env: &Env, p: &Council, s: &Subject) -> Option<std::result::Result<(), String>> {
    let m = p
        .branches
        .iter()
        .find(|b| b.repo == s.repo && b.branch == s.branch)?;
    Some(holds(env, p, Path::new(&s.repo), m))
}

/// A mark holds while every reflog entry since its head is a commit or an amend by the owner, and
/// the branch's upstream is none or a branch of origin.
fn holds(env: &Env, p: &Council, repo: &Path, m: &Branch) -> std::result::Result<(), String> {
    let e = |e: crate::error::Error| e.to_string();
    if let Some((r, merge)) = upstream(env, repo, &m.branch).map_err(e)?
        && (r != "origin" || !merge.starts_with("refs/heads/"))
    {
        return Err(format!("its upstream is {merge} on {r}"));
    }
    let mine = identities(env, p, repo);
    let r = format!("refs/heads/{}", m.branch);
    let log = out(
        env,
        repo,
        &["reflog", "show", "--format=%H%x00%gs", &r, "--"],
        Kind::Read,
    )
    .map_err(e)?;
    for line in log.lines() {
        let (sha, what) = line.split_once('\0').unwrap_or((line, ""));
        if sha == m.head {
            return Ok(());
        }
        if !(what.starts_with("commit: ") || what.starts_with("commit (amend): ")) {
            return Err(format!(
                "the branch moved by {}",
                what.split(':').next().unwrap_or(what)
            ));
        }
        let author = out(
            env,
            repo,
            &["log", "-1", "--format=%ae", sha, "--"],
            Kind::Read,
        )
        .map_err(e)?;
        if !mine.contains(&author.trim().to_string()) {
            return Err(format!("a commit by {}", author.trim()));
        }
    }
    Err("its reflog no longer reaches the marked head".into())
}

/// The owner's identities in `repo`: its `user.email` and the listed ones.
pub fn identities(env: &Env, p: &Council, repo: &Path) -> Vec<String> {
    let email = out(env, repo, &["config", "user.email"], Kind::Read).unwrap_or_default();
    let email = Some(email.trim().to_string()).filter(|e| !e.is_empty());
    email
        .into_iter()
        .chain(p.identities.iter().cloned())
        .collect()
}

/// `branch`'s upstream: its remote and the ref it merges.
fn upstream(env: &Env, repo: &Path, branch: &str) -> Result<Option<(String, String)>> {
    let get = |k: &str| {
        let r = runner::run(
            env,
            repo,
            &["config", &format!("branch.{branch}.{k}")],
            Kind::Read,
            Opts::default(),
        );
        r.map(|o| (o.code == 0).then(|| o.text().trim().to_string()))
    };
    Ok(get("remote")?.zip(get("merge")?))
}

/// A remote-tracking name split into its remote, the longest that matches, and branch.
fn split_remote(env: &Env, repo: &Path, name: &str) -> Result<(String, String)> {
    let remotes = out(env, repo, &["remote"], Kind::Read)?;
    let mut all: Vec<&str> = remotes.lines().collect();
    all.sort_by_key(|r| std::cmp::Reverse(r.len()));
    let found = all.iter().find_map(|r| {
        Some((
            r.to_string(),
            name.strip_prefix(r)?.strip_prefix('/')?.to_string(),
        ))
    });
    Ok(found.unwrap_or_else(|| (String::new(), name.into())))
}

/// Whether every fetch refspec of `remote` that can make `refs/remotes/<remote>/<branch>` maps from
/// `refs/heads/*`, and one does.
fn from_heads(env: &Env, repo: &Path, remote: &str, branch: &str) -> Result<bool> {
    let r = runner::run(
        env,
        repo,
        &["config", "--get-all", &format!("remote.{remote}.fetch")],
        Kind::Read,
        Opts::default(),
    )?;
    let target = format!("refs/remotes/{remote}/{branch}");
    let mut any = false;
    for spec in r.text().lines() {
        let (src, dst) = spec
            .trim_start_matches('+')
            .split_once(':')
            .unwrap_or((spec, ""));
        if glob(dst, &target) {
            if !src.starts_with("refs/heads/") {
                return Ok(false);
            }
            any = true;
        }
    }
    Ok(any)
}

/// A refspec side with at most one `*` against a full ref.
fn glob(pattern: &str, name: &str) -> bool {
    match pattern.split_once('*') {
        Some((a, b)) => name.len() > a.len() + b.len() && name.starts_with(a) && name.ends_with(b),
        None => pattern == name,
    }
}

/// The commits of base..head with their authors, and whether each is the owner's.
pub fn authors(env: &Env, p: &Council, s: &Subject) -> Result<Vec<(String, String, bool)>> {
    let repo = Path::new(&s.repo);
    let mine = identities(env, p, repo);
    let range = format!("{}..{}", s.base_sha, s.head_sha);
    let log = out(
        env,
        repo,
        &["log", "--format=%h%x00%an <%ae>%x00%ae", &range, "--"],
        Kind::Read,
    )?;
    let rows = log.lines().filter_map(|l| {
        let mut f = l.split('\0');
        let (sha, who, email) = (f.next()?, f.next()?, f.next()?);
        Some((sha.into(), who.into(), mine.iter().any(|m| m == email)))
    });
    Ok(rows.collect())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::git::testutil::Sandbox;

    struct T {
        sb: Sandbox,
        repo: PathBuf,
    }

    impl T {
        fn new() -> T {
            let sb = Sandbox::new();
            let repo = sb.clone_pair("r");
            T { sb, repo }
        }

        fn subject(&self, branch: &str, remote: bool) -> Subject {
            Subject {
                repo: self.repo.display().to_string(),
                branch: branch.into(),
                remote,
                base: "main".into(),
                base_sha: String::new(),
                head_sha: String::new(),
                of: Default::default(),
            }
        }

        fn trusted(&self, p: &Council, branch: &str, remote: bool) -> bool {
            classify(&self.sb.state, p, &self.subject(branch, remote))
                .unwrap()
                .trusted
        }

        fn git(&self, args: &[&str]) -> String {
            self.sb.git(&self.repo, args)
        }

        fn whole(&self) -> Council {
            Council {
                repos: vec![Mark {
                    name: self.repo.display().to_string(),
                    ..Mark::default()
                }],
                ..Council::default()
            }
        }
    }

    #[test]
    fn marking_a_repo_keeps_a_mark_that_covers_it_as_the_owner_set_it() {
        let t = T::new();
        let (core, repo) = (&t.sb.state, t.repo.display().to_string());
        let own = Mark {
            name: repo.clone(),
            remote: Some("upstream".into()),
            forks: true,
            ..Mark::default()
        };
        change(core, |p| p.repos.push(own.clone())).unwrap();
        mark_repo(core, &repo).unwrap();
        assert_eq!(load(core).repos, [own]);

        change(core, |p| p.repos.clear()).unwrap();
        let ws = t.repo.parent().unwrap().display().to_string();
        let work = core.save_workspace("Work".into(), None).unwrap();
        core.add_folder(Some(work), ws).unwrap();
        let work = Mark {
            name: "Work".into(),
            forks: true,
            ..Mark::default()
        };
        change(core, |p| p.workspaces.push(work)).unwrap();
        assert!(covers(core, &repo));
        mark_repo(core, &repo).unwrap();
        assert!(
            load(core).repos.is_empty(),
            "the workspace mark stays the one that counts"
        );

        change(core, |p| p.workspaces.clear()).unwrap();
        assert!(!covers(core, &repo));
        mark_repo(core, &repo).unwrap();
        assert_eq!(
            load(core).repos,
            [Mark {
                name: repo,
                ..Mark::default()
            }]
        );
    }

    #[test]
    fn a_trusted_repo_trusts_its_own_remote_s_branches_and_not_pull_refs() {
        let t = T::new();
        t.git(&[
            "config",
            "--add",
            "remote.origin.fetch",
            "+refs/pull/*/head:refs/remotes/origin/pr/*",
        ]);
        let p = t.whole();
        assert!(t.trusted(&p, "origin/main", true));
        assert!(!t.trusted(&p, "origin/pr/7", true));
        assert!(!t.trusted(&Council::default(), "origin/main", true));
        let ws = Council {
            workspaces: vec![Mark {
                name: "Work".into(),
                ..Mark::default()
            }],
            ..Council::default()
        };
        let dir = t.sb.dir.path().display().to_string();
        let work = t.sb.state.save_workspace("Work".into(), None).unwrap();
        t.sb.state.add_folder(Some(work), dir).unwrap();
        let why = classify(&t.sb.state, &ws, &t.subject("origin/main", true)).unwrap();
        assert_eq!(
            why,
            Trust {
                trusted: true,
                why: "trusted by workspace Work".into()
            }
        );
    }

    #[test]
    fn an_mr_is_trusted_from_the_own_remote_of_a_marked_repo_and_from_a_fork_only_with_forks() {
        let t = T::new();
        let mr = |remote: &str, fork: bool| Subject {
            of: Of::Mr {
                number: 7,
                title: "t".into(),
                author: "a".into(),
                url: String::new(),
                remote: remote.into(),
                head_ref: "refs/merge-requests/7/head".into(),
                fork,
            },
            ..t.subject("feature", true)
        };
        let trusted = |p: &Council, s: &Subject| classify(&t.sb.state, p, s).unwrap().trusted;
        let mut p = t.whole();
        assert!(trusted(&p, &mr("origin", false)), "a colleague's MR");
        assert!(!trusted(&p, &mr("origin", true)), "a fork's MR");
        assert!(!trusted(&p, &mr("upstream", false)), "another remote's");
        p.repos[0].forks = true;
        assert!(trusted(&p, &mr("origin", true)));
        assert!(!trusted(&Council::default(), &mr("origin", false)));
    }

    #[test]
    fn a_workspace_uses_its_chosen_remote_and_forks_setting() {
        let t = T::new();
        let core = &t.sb.state;
        let id = core.save_workspace("Work".into(), None).unwrap();
        core.add_folder(Some(id), t.sb.dir.path().display().to_string())
            .unwrap();
        core.set_block(
            BLOCK,
            serde_json::json!({
                "workspaces": [{"name": "Work", "remote": "upstream", "future": 7}],
                "future": true,
            }),
        )
        .unwrap();
        let mut subject = t.subject("feature", true);
        subject.of = Of::Mr {
            number: 7,
            title: "Feature".into(),
            author: "author".into(),
            url: String::new(),
            remote: "upstream".into(),
            head_ref: "refs/merge-requests/7/head".into(),
            fork: true,
        };
        assert!(!classify(core, &load(core), &subject).unwrap().trusted);
        change(core, |p| p.workspaces[0].forks = true).unwrap();
        assert!(classify(core, &load(core), &subject).unwrap().trusted);
        let saved = &core.get_settings().other[BLOCK];
        assert_eq!(saved["workspaces"][0]["future"], 7);
        assert_eq!(saved["future"], true);
    }

    #[test]
    fn in_a_trusted_repo_a_local_branch_is_trusted_unless_its_upstream_is_a_fork_s() {
        let t = T::new();
        let p = t.whole();
        t.git(&["branch", "mine"]);
        assert!(t.trusted(&p, "mine", false));
        assert!(t.trusted(&p, "main", false), "upstream origin/main");
        t.git(&["remote", "add", "fork", "https://example.com/fork.git"]);
        t.git(&["branch", "forked"]);
        t.git(&["config", "branch.forked.remote", "fork"]);
        t.git(&["config", "branch.forked.merge", "refs/heads/main"]);
        assert!(!t.trusted(&p, "forked", false));
        t.git(&["branch", "pr-7"]);
        t.git(&["config", "branch.pr-7.remote", "origin"]);
        t.git(&["config", "branch.pr-7.merge", "refs/pull/7/head"]);
        assert!(
            !t.trusted(&p, "pr-7", false),
            "gh pr checkout of a fork without maintainer edits"
        );
    }

    #[test]
    fn a_branch_mark_holds_through_commits_and_drops_on_anything_else() {
        let t = T::new();
        let repo = t.repo.clone();
        t.git(&["switch", "-qc", "feat"]);
        let head = t.sb.commit(&repo, "a", "1", "one");
        let p = Council {
            branches: vec![Branch {
                repo: repo.display().to_string(),
                branch: "feat".into(),
                head: head.clone(),
                ..Branch::default()
            }],
            ..Council::default()
        };
        assert!(!t.trusted(&Council::default(), "feat", false));
        assert!(t.trusted(&p, "feat", false));
        t.sb.commit(&repo, "a", "2", "two");
        t.git(&["commit", "-q", "--amend", "-m", "two!"]);
        t.git(&["push", "-q", "origin", "feat"]);
        assert!(t.trusted(&p, "feat", false), "commit, amend and push");
        t.git(&[
            "-c",
            "user.email=other@example.com",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "theirs",
        ]);
        assert!(!t.trusted(&p, "feat", false), "a commit by someone else");
        let mut aliases = p.clone();
        aliases.identities.push("other@example.com".into());
        assert!(t.trusted(&aliases, "feat", false));
        t.git(&["reset", "-q", "--hard", "HEAD~1"]);
        assert!(!t.trusted(&p, "feat", false), "a reset");
        let mut moved = p.clone();
        moved.branches[0].head = t.git(&["rev-parse", "HEAD"]);
        assert!(t.trusted(&moved, "feat", false));
        t.git(&["switch", "-qc", "side", "main"]);
        t.sb.commit(&repo, "b", "b", "side");
        t.git(&["switch", "-q", "feat"]);
        t.git(&["cherry-pick", "side"]);
        assert!(!t.trusted(&moved, "feat", false), "a cherry-pick");
        moved.branches[0].head = t.git(&["rev-parse", "HEAD"]);
        t.git(&["merge", "-q", "--no-ff", "-m", "m", "side"]);
        assert!(!t.trusted(&moved, "feat", false), "a merge");
        moved.branches[0].head = "0".repeat(40);
        assert!(
            !t.trusted(&moved, "feat", false),
            "a head the reflog does not reach"
        );
    }
}
