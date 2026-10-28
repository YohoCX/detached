//! Which git calls carry the account token (PLAN.md "Git runner"): hooks see it only when LFS is in play.

use std::path::{Path, PathBuf};

use crate::accounts;
use crate::git::checkout::checkout;
use crate::git::pull::pull;
use crate::git::testutil::Sandbox;
use crate::state::Cancel;
use crate::types::{AccountInput, CheckoutResult, CheckoutTarget, Provider};

/// A clone with an https remote that matches a stored account, and a hook that records `WB_*` env.
fn setup(sb: &Sandbox, hook: &str) -> (PathBuf, PathBuf) {
    let work = sb.clone_pair("work");
    sb.git(
        &work,
        &["remote", "add", "gh", "https://git.example.com/team/x.git"],
    );
    accounts::save(
        &sb.state,
        AccountInput {
            id: None,
            provider: Provider::GitlabSelfHosted,
            base_url: "https://git.example.com".into(),
            username: "oauth2".into(),
            label: "test".into(),
            token: Some("s3cret".into()),
        },
    )
    .unwrap();
    let log = sb.path("hook-env");
    sb.hook(
        &work,
        hook,
        &format!("{{ echo ran; env | grep '^WB_'; }} >> '{}'; true", log.display()),
    );
    (work, log)
}

fn seen(log: &Path) -> String {
    std::fs::read_to_string(log).unwrap_or_default()
}

fn switch(sb: &Sandbox, repo: &Path, name: &str) {
    let r = checkout(
        &sb.state,
        repo,
        &CheckoutTarget::Local { name: name.into() },
    )
    .unwrap();
    assert!(matches!(r, CheckoutResult::Switched { .. }), "{r:?}");
}

#[test]
fn hooks_get_no_token_without_lfs() {
    let sb = Sandbox::new();
    let (work, log) = setup(&sb, "post-checkout");
    sb.git(&work, &["branch", "b"]);
    switch(&sb, &work, "b");
    assert_eq!(seen(&log), "ran\n");
}

#[test]
fn switching_to_a_commit_that_adds_lfs_gets_the_token() {
    let sb = Sandbox::new();
    let (work, log) = setup(&sb, "post-checkout");
    sb.git(&work, &["switch", "-q", "-c", "lfs"]);
    sb.commit(&work, ".gitattributes", "*.bin filter=lfs\n", "track bin");
    sb.git(&work, &["switch", "-q", "main"]);
    std::fs::remove_file(&log).ok();
    switch(&sb, &work, "lfs");
    assert!(seen(&log).contains("WB_TOKEN_0=s3cret"), "{}", seen(&log));
}

#[test]
fn a_fast_forward_that_first_adds_lfs_gets_the_token() {
    let sb = Sandbox::new();
    let (work, log) = setup(&sb, "post-merge");
    let other = sb.clone_pair("other");
    sb.commit(&other, ".gitattributes", "*.bin filter=lfs\n", "track bin");
    sb.git(&other, &["push", "-q", "origin", "main"]);
    pull(&sb.state, &work, &Cancel::default(), &|_| {}).unwrap();
    assert!(seen(&log).contains("WB_TOKEN_0=s3cret"), "{}", seen(&log));
}
