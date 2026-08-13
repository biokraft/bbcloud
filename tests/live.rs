#![allow(clippy::unwrap_used)]

//! Live-API smoke tests. Skipped by default: they need `BB_LIVE_TEST=1`,
//! resolvable credentials, and `cargo test -- --ignored` (each test also
//! carries `#[ignore]`, belt and braces). See `CLAUDE.md` for why this file
//! exists — the mocked suite cannot detect an endpoint Atlassian retired.

use assert_cmd::Command;

/// The live preconditions, or `None` to skip. Never prompts, never fails
/// merely because credentials or env vars are absent — the caller is
/// responsible for printing why it skipped.
fn live_env() -> Option<String> {
    if std::env::var("BB_LIVE_TEST").ok().as_deref() != Some("1") {
        return None;
    }
    if bb_cli::credentials::load().is_err() {
        return None;
    }
    std::env::var("BB_WORKSPACE")
        .ok()
        .filter(|v| !v.trim().is_empty())
}

fn bb() -> Command {
    let mut cmd = Command::cargo_bin("bb").unwrap();
    cmd.env("NO_COLOR", "1");
    cmd
}

fn assert_not_retired(stderr: &str, code: Option<i32>) {
    assert_ne!(
        code,
        Some(3),
        "exited not-found — a retired endpoint signature"
    );
    let lower = stderr.to_lowercase();
    for needle in ["change-2770", "deprecated", "not found"] {
        assert!(
            !lower.contains(needle),
            "stderr contains {needle:?}, the signature of a retired endpoint: {stderr}"
        );
    }
}

#[test]
#[ignore]
fn pr_mine_author_all_states() {
    let Some(workspace) = live_env() else {
        eprintln!("skipping: set BB_LIVE_TEST=1, BB_WORKSPACE=<slug>, and resolvable credentials");
        return;
    };
    let assert = bb()
        .env("BB_WORKSPACE", &workspace)
        .args([
            "pr",
            "mine",
            "--role",
            "author",
            "--state",
            "all",
            "--json",
            "--repo-limit",
            "1",
        ])
        .assert();
    let output = assert.get_output().clone();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    assert_not_retired(&stderr, output.status.code());
    assert!(output.status.success(), "stderr: {stderr}");
    let value: serde_json::Value = serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|e| panic!("stdout did not parse as JSON: {e}; stdout was {output:?}"));
    assert!(
        value.get("pull_requests").is_some(),
        "missing pull_requests: {value}"
    );
    assert!(value.get("partial").is_some(), "missing partial: {value}");
}

#[test]
#[ignore]
fn pr_mine_reviewer() {
    let Some(workspace) = live_env() else {
        eprintln!("skipping: set BB_LIVE_TEST=1, BB_WORKSPACE=<slug>, and resolvable credentials");
        return;
    };
    let assert = bb()
        .env("BB_WORKSPACE", &workspace)
        .args([
            "pr",
            "mine",
            "--role",
            "reviewer",
            "--repo-limit",
            "3",
            "--json",
        ])
        .assert();
    let output = assert.get_output().clone();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    assert_not_retired(&stderr, output.status.code());
    assert!(output.status.success(), "stderr: {stderr}");
    let value: serde_json::Value = serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|e| panic!("stdout did not parse as JSON: {e}; stdout was {output:?}"));
    assert!(
        value.get("pull_requests").is_some(),
        "missing pull_requests: {value}"
    );
    assert!(value.get("partial").is_some(), "missing partial: {value}");
}

#[test]
#[ignore]
fn pr_list_build_status() {
    if live_env().is_none() {
        eprintln!("skipping: set BB_LIVE_TEST=1, BB_WORKSPACE=<slug>, and resolvable credentials");
        return;
    }
    let Some(repo) = std::env::var("BB_LIVE_REPO")
        .ok()
        .filter(|v| !v.trim().is_empty())
    else {
        eprintln!("skipping: set BB_LIVE_REPO=<workspace>/<repo> to exercise pr list --build");
        return;
    };
    let assert = bb()
        .args(["pr", "list", "-R", &repo, "--build", "--json"])
        .assert();
    let output = assert.get_output().clone();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    assert_not_retired(&stderr, output.status.code());
    assert!(output.status.success(), "stderr: {stderr}");
    let _value: serde_json::Value = serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|e| panic!("stdout did not parse as JSON: {e}; stdout was {output:?}"));
}
