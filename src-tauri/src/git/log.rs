//! Streamed `git log` with lanes, and the commit queries around it.

use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::Ordering;

use crate::env::Env;
use crate::error::{Error, Result};
use crate::git::arg;
use crate::git::graph::Graph;
use crate::git::runner::{Kind, Opts, out, run};
use crate::git::status::commits;
use crate::state::Cancel;
use crate::types::*;

const BATCH: usize = 500;
const CAP: usize = 100_000;

/// Streams `query`'s rows in batches, then `Done` (or `Error`).
pub fn log(
    env: &Env,
    repo: &Path,
    query: &LogQuery,
    query_id: u64,
    cancel: &Cancel,
    emit: &(dyn Fn(LogEvent) + Sync),
) {
    emit(match stream(env, repo, query, query_id, cancel, emit) {
        Ok(truncated) => LogEvent::Done {
            query_id,
            truncated,
        },
        Err(Error::Cancelled) => LogEvent::Done {
            query_id,
            truncated: false,
        },
        Err(e) => LogEvent::Error {
            query_id,
            error: e.to_string(),
        },
    });
}

/// Whether the cap cut the history short.
fn stream(
    env: &Env,
    repo: &Path,
    q: &LogQuery,
    query_id: u64,
    cancel: &Cancel,
    emit: &(dyn Fn(LogEvent) + Sync),
) -> Result<bool> {
    let mut args = vec![
        "log".into(),
        "--date-order".into(),
        format!("--format={LOG_FORMAT}"),
    ];
    match &q.refs {
        LogRefs::All => args.extend(["--branches", "--remotes", "--tags", "HEAD"].map(Into::into)),
        LogRefs::Head => args.push("HEAD".into()),
        LogRefs::Branch { name } => args.push(arg(name)?.into()),
    }
    let text = q.text.as_deref().filter(|t| !t.is_empty());
    let author = q.author.as_deref().filter(|a| !a.is_empty());
    // A short hash would also be a word: "defaced" is hex.
    let hash = text
        .filter(|t| (7..=40).contains(&t.len()) && t.bytes().all(|b| b.is_ascii_hexdigit()))
        .map(str::to_ascii_lowercase);
    if let (Some(t), None) = (text, &hash) {
        args.extend(["-i".into(), format!("--grep={t}")]);
    }
    if let Some(a) = author {
        args.push(format!("--author={a}"));
    }
    if q.path.is_some() {
        args.push("--parents".into());
    }
    args.push("--".into());
    args.extend(q.path.clone());

    let graph = (text.is_none() && author.is_none()).then(Graph::default);
    let st = Mutex::new((graph, Vec::new(), 0, false));
    let on_line = |l: &[u8]| {
        let (graph, batch, read, truncated) = &mut *st.lock().unwrap();
        if *read == CAP {
            *truncated = true;
            return false;
        }
        let Some(mut row) = parse(l) else {
            return true;
        };
        *read += 1;
        if hash.as_ref().is_some_and(|h| !row.sha.starts_with(h)) {
            return true;
        }
        if let Some(g) = graph {
            g.place(&mut row);
        }
        batch.push(row);
        if batch.len() < BATCH {
            return true;
        }
        if cancel.load(Ordering::SeqCst) {
            return false;
        }
        let rows = std::mem::take(batch);
        emit(LogEvent::Rows { query_id, rows });
        true
    };
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let opts = Opts {
        cancel: Some(cancel),
        lines: Some(&on_line),
        ..Default::default()
    };
    let o = run(env, repo, &args, Kind::Read, opts)?;
    if cancel.load(Ordering::SeqCst) {
        return Err(Error::Cancelled);
    }
    o.ok()?;
    let (_, rows, _, truncated) = st.into_inner().unwrap();
    if !rows.is_empty() {
        emit(LogEvent::Rows { query_id, rows });
    }
    Ok(truncated)
}

/// One `LOG_FORMAT` line.
fn parse(l: &[u8]) -> Option<LogRow> {
    let l = String::from_utf8_lossy(l);
    let f: Vec<&str> = l.splitn(7, '\x1f').collect();
    let [sha, parents, author, email, time, refs, subject] = f[..] else {
        return None;
    };
    let words = |s: &str, sep| {
        s.split(sep)
            .filter(|w: &&str| !w.is_empty())
            .map(String::from)
            .collect()
    };
    Some(LogRow {
        sha: sha.into(),
        parents: words(parents, " "),
        author: author.into(),
        email: email.into(),
        time: time.parse().ok()?,
        refs: words(refs, ", "),
        subject: subject.into(),
        column: 0,
        color: 0,
        top: vec![],
        bottom: vec![],
    })
}

pub fn commit_details(env: &Env, repo: &Path, sha: &str) -> Result<CommitDetails> {
    let format = "--format=%H%x00%P%x00%an%x00%ae%x00%at%x00%cn%x00%ce%x00%ct%x00%B";
    let s = out(
        env,
        repo,
        &["log", "-1", format, arg(sha)?, "--"],
        Kind::Read,
    )?;
    let f: Vec<&str> = s.splitn(9, '\0').collect();
    let [
        sha,
        parents,
        author,
        author_email,
        at,
        committer,
        committer_email,
        ct,
        message,
    ] = f[..]
    else {
        return Err(Error::Other(format!("no commit {sha}")));
    };
    Ok(CommitDetails {
        sha: sha.into(),
        parents: parents.split_whitespace().map(String::from).collect(),
        author: author.into(),
        author_email: author_email.into(),
        author_time: at.parse().unwrap_or_default(),
        committer: committer.into(),
        committer_email: committer_email.into(),
        committer_time: ct.parse().unwrap_or_default(),
        message: message.trim_end().into(),
    })
}

/// `git branch -a --contains`, without the detached HEAD entry and symbolic refs like `origin/HEAD`.
pub fn branches_containing(env: &Env, repo: &Path, sha: &str) -> Result<Vec<String>> {
    let args = [
        "branch",
        "-a",
        "--contains",
        arg(sha)?,
        "--format=%(refname:short)\t%(symref)",
    ];
    Ok(out(env, repo, &args, Kind::Read)?
        .lines()
        .filter_map(|l| l.strip_suffix('\t'))
        .filter(|b| !b.starts_with('('))
        .map(String::from)
        .collect())
}

pub fn merge_base(env: &Env, repo: &Path, a: &str, b: &str) -> Result<String> {
    let o = run(
        env,
        repo,
        &["merge-base", arg(a)?, arg(b)?],
        Kind::Read,
        Opts::default(),
    )?;
    if o.code == 1 {
        return Err(Error::Other(format!("{a} and {b} have no common commit")));
    }
    Ok(o.ok()?.text().trim().into())
}

/// `git log base..head`.
pub fn range_log(env: &Env, repo: &Path, base: &str, head: &str) -> Result<Vec<CommitSummary>> {
    commits(
        env,
        repo,
        &[&format!("{}..{}", arg(base)?, arg(head)?), "--"],
    )
}

/// Hunks of `git diff -U3 -M --diff-algorithm=myers <base> <head> -- <orig_path> <path>`.
pub fn hunk_ranges(
    env: &Env,
    repo: &Path,
    base: &str,
    head: &str,
    path: &str,
    orig_path: Option<&str>,
) -> Result<Vec<Hunk>> {
    let mut args = vec![
        "diff",
        "-U3",
        "-M",
        "--diff-algorithm=myers",
        arg(base)?,
        arg(head)?,
    ];
    args.push("--");
    args.extend(orig_path);
    args.push(path);
    Ok(out(env, repo, &args, Kind::Read)?
        .lines()
        .filter_map(hunk)
        .collect())
}

/// `@@ -1,3 +1,4 @@`; a missing count is 1.
fn hunk(l: &str) -> Option<Hunk> {
    let (old, new) = l
        .strip_prefix("@@ -")?
        .split_once(" @@")?
        .0
        .split_once(" +")?;
    let range = |s: &str| -> Option<(u32, u32)> {
        let (start, n) = s.split_once(',').unwrap_or((s, "1"));
        Some((start.parse().ok()?, n.parse().ok()?))
    };
    let ((old_start, old_lines), (new_start, new_lines)) = (range(old)?, range(new)?);
    Some(Hunk {
        old_start,
        old_lines,
        new_start,
        new_lines,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::testutil::Sandbox;
    use std::io::Write;
    use std::path::PathBuf;
    use std::process::{Command, Stdio};
    use std::time::Instant;

    /// `n` commits on main, each tenth also merging the commit five back, through fast-import.
    fn many(sb: &Sandbox, n: usize) -> PathBuf {
        let repo = sb.path("many");
        sb.git(sb.dir.path(), &["init", "-q", "many"]);
        let mut s = String::new();
        for i in 1..=n {
            s += &format!(
                "commit refs/heads/main\nmark :{i}\ncommitter T <t@e> {} +0000\ndata 1\nc\n",
                1_000_000_000 + i
            );
            if i > 1 {
                s += &format!("from :{}\n", i - 1);
            }
            if i % 10 == 0 {
                s += &format!("merge :{}\n", i - 5);
            }
        }
        let mut c = Command::new(&sb.state.env().git)
            .arg("-C")
            .arg(&repo)
            .args(["fast-import", "--quiet"])
            .stdin(Stdio::piped())
            .spawn()
            .unwrap();
        c.stdin.take().unwrap().write_all(s.as_bytes()).unwrap();
        assert!(c.wait().unwrap().success());
        repo
    }

    fn query(refs: LogRefs) -> LogQuery {
        LogQuery {
            refs,
            path: None,
            text: None,
            author: None,
        }
    }

    fn events(sb: &Sandbox, repo: &Path, q: &LogQuery) -> Vec<LogEvent> {
        let events = Mutex::new(vec![]);
        log(&sb.state.env(), repo, q, 7, &Cancel::default(), &|e| {
            events.lock().unwrap().push(e)
        });
        events.into_inner().unwrap()
    }

    /// The rows of one query, and whether it was truncated.
    fn rows(sb: &Sandbox, repo: &Path, q: &LogQuery) -> (Vec<LogRow>, bool) {
        let mut rows = vec![];
        for e in events(sb, repo, q) {
            match e {
                LogEvent::Rows { rows: r, .. } => rows.extend(r),
                LogEvent::Done { truncated, .. } => return (rows, truncated),
                LogEvent::Error { error, .. } => panic!("{error}"),
            }
        }
        panic!("no Done")
    }

    /// a ← b ← d ← m on main, c on feature from b, m merges c; o on other from a.
    struct Repo {
        sb: Sandbox,
        path: PathBuf,
        a: String,
        b: String,
        c: String,
        d: String,
        m: String,
        o: String,
    }

    fn repo() -> Repo {
        let sb = Sandbox::new();
        sb.git(sb.dir.path(), &["init", "-q", "r"]);
        let path = sb.path("r");
        let r = &path;
        let a = sb.commit(r, "x", "1\n", "add x");
        sb.git(r, &["tag", "v1"]);
        sb.git(r, &["branch", "other"]);
        let b = sb.commit(r, "y", "1\n", "Fix y");
        sb.git(r, &["switch", "-qc", "feature"]);
        std::fs::write(r.join("x"), "2\n").unwrap();
        sb.git(r, &["commit", "-qam", "feature x", "--author=Other <o@e>"]);
        let c = sb.git(r, &["rev-parse", "HEAD"]);
        sb.git(r, &["switch", "-q", "other"]);
        let o = sb.commit(r, "o", "1\n", "other o");
        sb.git(r, &["switch", "-q", "main"]);
        let d = sb.commit(r, "z", "1\n", "main z");
        sb.git(
            r,
            &["merge", "-q", "--no-ff", "-m", "merge feature", "feature"],
        );
        let m = sb.git(r, &["rev-parse", "HEAD"]);
        Repo {
            sb,
            path,
            a,
            b,
            c,
            d,
            m,
            o,
        }
    }

    fn shas(rows: &[LogRow]) -> Vec<String> {
        rows.iter().map(|r| r.sha.clone()).collect()
    }

    #[test]
    fn all_refs_in_date_order_with_refs_and_lanes() {
        let r = repo();
        let (rows, truncated) = rows(&r.sb, &r.path, &query(LogRefs::All));
        let args = ["log", "--date-order", "--format=%H", "--branches", "HEAD"];
        let want = r.sb.git(&r.path, &args);
        assert_eq!(shas(&rows), want.lines().collect::<Vec<_>>());
        assert!(!truncated);
        let row = |sha: &str| rows.iter().find(|x| x.sha == sha).unwrap();
        assert_eq!(row(&r.m).refs, ["HEAD -> main"]);
        assert_eq!(row(&r.m).parents, [r.d.clone(), r.c.clone()]);
        assert_eq!(row(&r.c).refs, ["feature"]);
        assert_eq!(row(&r.a).refs, ["tag: v1"]);
        assert_eq!(row(&r.c).author, "Other");
        assert_eq!(row(&r.m).subject, "merge feature");
        assert_eq!(row(&r.m).bottom.len(), 2);
        assert_eq!(row(&r.a).top.len(), 2);
    }

    #[test]
    fn branch_and_head_filters() {
        let r = repo();
        let feature = LogRefs::Branch {
            name: "feature".into(),
        };
        assert_eq!(
            shas(&rows(&r.sb, &r.path, &query(feature)).0),
            [r.c.as_str(), r.b.as_str(), r.a.as_str()]
        );
        let head = shas(&rows(&r.sb, &r.path, &query(LogRefs::Head)).0);
        assert_eq!(head.len(), 5);
        assert!(!head.contains(&r.o));
        let bad = query(LogRefs::Branch {
            name: "nope".into(),
        });
        assert!(matches!(
            &events(&r.sb, &r.path, &bad)[..],
            [LogEvent::Error { query_id: 7, .. }]
        ));
    }

    #[test]
    fn text_hash_and_author_filters_have_no_graph() {
        let r = repo();
        let filtered = |text: Option<&str>, author: Option<&str>| {
            let q = LogQuery {
                text: text.map(Into::into),
                author: author.map(Into::into),
                ..query(LogRefs::All)
            };
            let rows = rows(&r.sb, &r.path, &q).0;
            assert!(rows.iter().all(|x| x.top.is_empty() && x.bottom.is_empty()));
            shas(&rows)
        };
        assert_eq!(filtered(Some("fix"), None), [r.b.as_str()]);
        assert_eq!(
            filtered(Some(&r.c[..8].to_uppercase()), None),
            [r.c.as_str()]
        );
        assert_eq!(filtered(None, Some("Other")), [r.c.as_str()]);
        assert_eq!(filtered(Some("ADD"), Some("Test")), [r.a.as_str()]);
    }

    #[test]
    fn path_filter_rewrites_parents_and_keeps_the_graph() {
        let r = repo();
        let q = LogQuery {
            path: Some("x".into()),
            ..query(LogRefs::All)
        };
        let rows = rows(&r.sb, &r.path, &q).0;
        // m takes x from c, so it is simplified away; c's parent b never touched x.
        assert_eq!(shas(&rows), [r.c.as_str(), r.a.as_str()]);
        assert_eq!(rows[0].parents, [r.a.as_str()]);
        let edges = |e: &[Edge]| e.iter().map(|e| (e.from, e.to)).collect::<Vec<_>>();
        assert_eq!(edges(&rows[0].bottom), [(0, 0)]);
        assert_eq!(edges(&rows[1].top), [(0, 0)]);
    }

    #[test]
    fn details_branches_merge_base_and_range() {
        let r = repo();
        let env = r.sb.state.env();
        let d = commit_details(&env, &r.path, &r.m[..10]).unwrap();
        assert_eq!(
            (d.sha, d.parents, d.message, d.author),
            (
                r.m.clone(),
                vec![r.d.clone(), r.c.clone()],
                "merge feature".into(),
                "Test".into()
            )
        );
        r.sb.git(&r.path, &["switch", "-q", "--detach", &r.c]);
        assert_eq!(
            branches_containing(&env, &r.path, &r.c).unwrap(),
            ["feature", "main"]
        );
        assert_eq!(merge_base(&env, &r.path, &r.d, &r.c).unwrap(), r.b);
        let mut range: Vec<_> = range_log(&env, &r.path, &r.a, &r.m).unwrap();
        range.sort_by(|x, y| x.sha.cmp(&y.sha));
        let mut want = [r.b.as_str(), r.c.as_str(), r.d.as_str(), r.m.as_str()];
        want.sort();
        assert_eq!(range.iter().map(|c| &c.sha).collect::<Vec<_>>(), want);
        assert_eq!(range_log(&env, &r.path, &r.c, &r.m).unwrap().len(), 2);
    }

    #[test]
    fn hunks_pair_a_renamed_file_through_both_paths() {
        let sb = Sandbox::new();
        sb.git(sb.dir.path(), &["init", "-q", "r"]);
        let r = &sb.path("r");
        let text: String = (1..=20).map(|i| format!("{i}\n")).collect();
        let base = sb.commit(r, "long", &text, "long");
        let changed = text
            .replacen("2\n", "two\n", 1)
            .replace("18\n", "eighteen\n");
        std::fs::write(r.join("long"), changed).unwrap();
        sb.git(r, &["mv", "long", "long2"]);
        sb.git(r, &["commit", "-qam", "rename"]);
        let h = |old_start, old_lines, new_start, new_lines| Hunk {
            old_start,
            old_lines,
            new_start,
            new_lines,
        };
        let env = sb.state.env();
        let hunks = hunk_ranges(&env, r, &base, "HEAD", "long2", Some("long")).unwrap();
        assert_eq!(hunks, [h(1, 5, 1, 5), h(15, 6, 15, 6)]);
        let alone = hunk_ranges(&env, r, &base, "HEAD", "long2", None).unwrap();
        assert_eq!(alone, [h(0, 0, 1, 20)]);
    }

    #[test]
    fn a_new_query_drops_the_old_ones_batches() {
        let sb = Sandbox::new();
        let repo = many(&sb, 3000);
        let all = query(LogRefs::All);
        let seen = Mutex::new(vec![]);
        let emit = |e: LogEvent| {
            if let LogEvent::Rows { query_id: 1, .. } = e {
                // What the UI does when the next query starts.
                sb.state.ops.cancel(1);
            }
            seen.lock().unwrap().push(e);
        };
        let env = sb.state.env();
        sb.state.op(1, |c| log(&env, &repo, &all, 1, c, &emit));
        sb.state.op(2, |c| log(&env, &repo, &all, 2, c, &emit));
        let seen = seen.into_inner().unwrap();
        let rows = |id: u64| -> usize {
            seen.iter()
                .map(|e| match e {
                    LogEvent::Rows { query_id, rows } if *query_id == id => rows.len(),
                    _ => 0,
                })
                .sum()
        };
        assert_eq!((rows(1), rows(2)), (BATCH, 3000));
        let done = |query_id| LogEvent::Done {
            query_id,
            truncated: false,
        };
        assert_eq!(seen[1], done(1));
        assert_eq!(seen.last(), Some(&done(2)));
    }

    #[test]
    fn a_hundred_thousand_commits_stream_with_lanes_and_the_cap_stops_git() {
        let sb = Sandbox::new();
        let repo = many(&sb, CAP + 500);
        let start = Instant::now();
        let (rows, truncated) = rows(&sb, &repo, &query(LogRefs::All));
        let took = start.elapsed();
        assert_eq!((rows.len(), truncated), (CAP, true));
        // Every lane leaving a row's bottom enters the next row's top.
        for w in rows.windows(2) {
            let mut below: Vec<u32> = w[0].bottom.iter().map(|e| e.to).collect();
            let mut above: Vec<u32> = w[1].top.iter().map(|e| e.from).collect();
            above.sort();
            below.sort();
            below.dedup();
            assert_eq!(below, above, "{}", w[1].sha);
        }
        eprintln!("100,000 commits in {took:?}");
        assert!(took.as_secs_f64() < 3.0, "{took:?}");
    }
}
