#![allow(clippy::unwrap_used)] // test code is exempt from the unwrap/expect ban

use assert_cmd::Command;
use predicates::str::contains;

fn bb() -> Command {
    let mut cmd = Command::cargo_bin("bb").unwrap();
    cmd.env("BB_EMAIL", "dev@example.com")
        .env("BB_TOKEN", "t0ken-value")
        .env("BB_REPO", "acme/widgets")
        .env("NO_COLOR", "1");
    cmd
}

#[test]
fn browse_url_prints_the_repository_url() {
    bb().args(["browse", "--print"])
        .assert()
        .success()
        .stdout(contains("https://bitbucket.org/acme/widgets"));
}

#[test]
fn browse_pr_targets_the_pull_request_page() {
    bb().args(["browse", "--print", "--pr", "7"])
        .assert()
        .success()
        .stdout(contains(
            "https://bitbucket.org/acme/widgets/pull-requests/7",
        ));
}

/// A malicious remote must be rejected while parsing, long before any process
/// is spawned, so no shell metacharacter can reach a command line.
#[test]
fn hostile_repo_value_is_rejected() {
    let mut cmd = Command::cargo_bin("bb").unwrap();
    cmd.env("BB_EMAIL", "dev@example.com")
        .env("BB_TOKEN", "t0ken-value")
        .env("NO_COLOR", "1")
        .args(["browse", "--print", "--repo", "acme/widgets;id"])
        .assert()
        .failure()
        .stderr(contains("invalid repository"));
}

#[test]
fn completions_emit_a_bash_script() {
    bb().args(["completions", "bash"])
        .assert()
        .success()
        .stdout(contains("_bb"));
}

#[test]
fn completions_emit_a_zsh_script() {
    bb().args(["completions", "zsh"])
        .assert()
        .success()
        .stdout(contains("#compdef bb"));
}
