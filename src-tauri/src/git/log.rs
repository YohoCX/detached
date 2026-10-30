//! Stubs returning fixtures until the streamed log, details and lanes exist.

use std::path::Path;

use serde_json::{Value, json};

use crate::env::Env;
use crate::error::Result;
use crate::state::Cancel;
use crate::types::*;

/// Streams `query`'s rows in batches, then `Done`.
pub fn log(
    _env: &Env,
    _repo: &Path,
    _query: &LogQuery,
    query_id: u64,
    _cancel: &Cancel,
    emit: &(dyn Fn(LogEvent) + Sync),
) {
    // d merges b (column 0) and c (column 1), both children of a.
    let row = |sha: &str,
               parents: &[&str],
               refs: &[&str],
               column: u32,
               top: Value,
               bottom: Value| {
        json!({"sha": sha.repeat(40), "parents": parents.iter().map(|p| p.repeat(40)).collect::<Vec<_>>(),
            "author": "Test", "email": "test@example.com", "time": 1_700_000_000, "refs": refs,
            "subject": format!("commit {sha}"), "column": column, "color": column, "top": top, "bottom": bottom})
    };
    let e = |from: u32, to: u32, color: u32| json!({"from": from, "to": to, "color": color});
    let rows = serde_json::from_value(json!([
        row(
            "d",
            &["b", "c"],
            &["HEAD -> main", "origin/main"],
            0,
            json!([]),
            json!([e(0, 0, 0), e(0, 1, 1)])
        ),
        row(
            "c",
            &["a"],
            &["feature"],
            1,
            json!([e(0, 0, 0), e(1, 1, 1)]),
            json!([e(0, 0, 0), e(1, 1, 1)])
        ),
        row(
            "b",
            &["a"],
            &[],
            0,
            json!([e(0, 0, 0), e(1, 1, 1)]),
            json!([e(0, 0, 0), e(1, 1, 1)])
        ),
        row(
            "a",
            &[],
            &["tag: v1"],
            0,
            json!([e(0, 0, 0), e(1, 0, 1)]),
            json!([])
        ),
    ]))
    .unwrap();
    emit(LogEvent::Rows { query_id, rows });
    emit(LogEvent::Done {
        query_id,
        truncated: false,
    });
}

pub fn commit_details(_env: &Env, _repo: &Path, sha: &str) -> Result<CommitDetails> {
    Ok(CommitDetails {
        sha: sha.into(),
        parents: vec![],
        author: "Test".into(),
        author_email: "test@example.com".into(),
        author_time: 1_700_000_000,
        committer: "Test".into(),
        committer_email: "test@example.com".into(),
        committer_time: 1_700_000_000,
        message: "commit\n\nBody.\n".into(),
    })
}

/// `git branch -a --contains`.
pub fn branches_containing(_env: &Env, _repo: &Path, _sha: &str) -> Result<Vec<String>> {
    Ok(vec!["main".into()])
}

pub fn merge_base(_env: &Env, _repo: &Path, a: &str, _b: &str) -> Result<String> {
    Ok(a.into())
}

/// `git log base..head`.
pub fn range_log(_env: &Env, _repo: &Path, _base: &str, head: &str) -> Result<Vec<CommitSummary>> {
    Ok(vec![CommitSummary {
        sha: head.into(),
        subject: "commit".into(),
        author: "Test".into(),
        time: 1_700_000_000,
    }])
}

/// Hunks of `git diff -U3 -M --diff-algorithm=myers <base> <head> -- <orig_path> <path>`.
pub fn hunk_ranges(
    _env: &Env,
    _repo: &Path,
    _base: &str,
    _head: &str,
    _path: &str,
    _orig_path: Option<&str>,
) -> Result<Vec<Hunk>> {
    Ok(vec![Hunk {
        old_start: 1,
        old_lines: 3,
        new_start: 1,
        new_lines: 4,
    }])
}
