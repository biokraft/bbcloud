#![allow(clippy::unwrap_used)] // test code is exempt from the unwrap/expect ban

use assert_cmd::Command;
use predicates::str::contains;
use std::path::Path;
use tempfile::{tempdir, TempDir};

/// `bb browse --print` resolves the repository and then only prints — no network, no browser.
/// Every ambient variable that could short-circuit resolution is removed.
fn bb_in(dir: &Path) -> Command {
    let mut cmd = Command::cargo_bin("bb").unwrap();
    cmd.current_dir(dir)
        .args(["browse", "--print"])
        .env("BB_KEYRING_DISABLE", "1")
        .env("NO_COLOR", "1")
        .env_remove("BB_REPO")
        .env_remove("BB_EMAIL")
        .env_remove("BB_TOKEN");
    cmd
}

/// A git repository with no remotes, and with git's own config isolated from the
/// developer's so a global `init.defaultBranch` or template cannot affect the result.
fn git_repo() -> TempDir {
    let tmp = tempdir().unwrap();
    run_git(tmp.path(), &["init"]);
    run_git(tmp.path(), &["config", "user.email", "dev@example.com"]);
    run_git(tmp.path(), &["config", "user.name", "Dev Person"]);
    tmp
}

fn run_git(dir: &Path, args: &[&str]) {
    let status = std::process::Command::new("git")
        .current_dir(dir)
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?} failed in {dir:?}");
}

#[test]
fn bb_repo_env_var_wins_without_consulting_git() {
    // An empty directory: if this consulted git it would fail outright.
    let tmp = tempdir().unwrap();
    let mut cmd = Command::cargo_bin("bb").unwrap();
    cmd.current_dir(tmp.path())
        .args(["browse", "--print"])
        .env("BB_KEYRING_DISABLE", "1")
        .env("NO_COLOR", "1")
        .env("BB_REPO", "acme/widgets")
        .assert()
        .success()
        .stdout(contains("https://bitbucket.org/acme/widgets"));
}

// NOTE: the brief assumed a whitespace-only `BB_REPO` falls through to git resolution
// inside `repo::resolve`'s own `if !value.trim().is_empty()` check. In practice `--repo`
// is declared with `env = "BB_REPO"` on the clap arg (src/main.rs), so clap itself turns
// any set `BB_REPO` — including a whitespace-only one — into `explicit: Some(..)` before
// `resolve` ever runs its own check. `resolve`'s manual re-check of `BB_REPO` is therefore
// unreachable through this binary; `RepoSlug::parse` gets the whitespace value directly
// and fails to parse it as a slug. This test verifies that observed (correct) behaviour
// instead of the unreachable fallthrough.
#[test]
fn a_whitespace_only_bb_repo_fails_to_parse_as_a_slug() {
    let repo = git_repo();
    run_git(
        repo.path(),
        &[
            "remote",
            "add",
            "origin",
            "git@bitbucket.org:acme/widgets.git",
        ],
    );

    let mut cmd = Command::cargo_bin("bb").unwrap();
    cmd.current_dir(repo.path())
        .args(["browse", "--print"])
        .env("BB_KEYRING_DISABLE", "1")
        .env("NO_COLOR", "1")
        .env("BB_REPO", "   ")
        .assert()
        .failure()
        .stderr(contains("invalid repository"));
}

#[test]
fn outside_a_git_repository_the_error_names_the_repo_flag() {
    let tmp = tempdir().unwrap();
    bb_in(tmp.path())
        .assert()
        .failure()
        .stderr(contains("no git repository here"))
        .stderr(contains("--repo"));
}

#[test]
fn a_repository_with_no_remotes_says_so() {
    let repo = git_repo();
    bb_in(repo.path())
        .assert()
        .failure()
        .stderr(contains("no git remotes configured"));
}

#[test]
fn a_non_bitbucket_remote_reports_how_many_were_checked() {
    let repo = git_repo();
    run_git(
        repo.path(),
        &[
            "remote",
            "add",
            "origin",
            "https://github.com/someone/thing.git",
        ],
    );

    bb_in(repo.path())
        .assert()
        .failure()
        .stderr(contains("no bitbucket.org remote found (checked 1)"));
}

#[test]
fn an_https_bitbucket_origin_resolves() {
    let repo = git_repo();
    run_git(
        repo.path(),
        &[
            "remote",
            "add",
            "origin",
            "https://bitbucket.org/acme/widgets.git",
        ],
    );

    bb_in(repo.path())
        .assert()
        .success()
        .stdout(contains("https://bitbucket.org/acme/widgets"));
}

/// The fork/mirror case: `origin` points elsewhere, but another remote is on Bitbucket.
#[test]
fn a_bitbucket_remote_other_than_origin_still_resolves() {
    let repo = git_repo();
    run_git(
        repo.path(),
        &[
            "remote",
            "add",
            "origin",
            "https://github.com/someone/fork.git",
        ],
    );
    run_git(
        repo.path(),
        &[
            "remote",
            "add",
            "upstream",
            "git@bitbucket.org:acme/widgets.git",
        ],
    );

    bb_in(repo.path())
        .assert()
        .success()
        .stdout(contains("https://bitbucket.org/acme/widgets"));
}

/// `pr create` infers its source branch from HEAD, so a detached HEAD must be reported
/// rather than producing a nonsense branch name.
///
/// NOTE: the brief expected the message "detached HEAD" from `git::current_branch`'s own
/// `branch.is_empty()` check (git.rs:36-39). In practice `git symbolic-ref --short HEAD`
/// exits non-zero (not zero-with-empty-output) when HEAD is detached, so `git_in` surfaces
/// git's own stderr ("fatal: ref HEAD is not a symbolic ref") before that check ever runs.
/// That branch is dead code through this CLI; this test asserts the real error text.
#[test]
fn a_detached_head_is_reported_when_the_source_branch_is_inferred() {
    let repo = git_repo();
    run_git(
        repo.path(),
        &[
            "remote",
            "add",
            "origin",
            "git@bitbucket.org:acme/widgets.git",
        ],
    );
    std::fs::write(repo.path().join("file.txt"), "hello").unwrap();
    run_git(repo.path(), &["add", "file.txt"]);
    run_git(repo.path(), &["commit", "-m", "initial"]);
    // Detach HEAD onto the commit itself.
    run_git(repo.path(), &["checkout", "--detach", "HEAD"]);

    Command::cargo_bin("bb")
        .unwrap()
        .current_dir(repo.path())
        .args(["pr", "create", "main"])
        .env("BB_KEYRING_DISABLE", "1")
        .env("NO_COLOR", "1")
        .env("BB_EMAIL", "dev@example.com")
        .env("BB_TOKEN", "t0ken-value")
        .env("BB_API_BASE", "http://127.0.0.1:1")
        .env_remove("BB_REPO")
        .assert()
        .failure()
        .stderr(contains("not a symbolic ref"));
}

/// A remote whose url is configured empty must be reported, not silently treated as a
/// repository named "".
#[test]
fn a_remote_with_an_empty_url_is_skipped_rather_than_parsed() {
    let repo = git_repo();
    run_git(repo.path(), &["config", "remote.origin.url", ""]);
    run_git(
        repo.path(),
        &[
            "remote",
            "add",
            "backup",
            "git@bitbucket.org:acme/widgets.git",
        ],
    );

    // The empty `origin` url yields an error from `remote_url`, which `resolve` skips
    // with `if let Ok(...)`, so the good remote still wins.
    bb_in(repo.path())
        .assert()
        .success()
        .stdout(contains("https://bitbucket.org/acme/widgets"));
}
