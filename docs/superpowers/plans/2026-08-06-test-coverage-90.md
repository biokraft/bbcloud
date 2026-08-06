# Raising line coverage to 90% — implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Raise `bbcloud` line coverage from a measured 84.64% to at least 90% by adding tests only.

**Architecture:** Five independent test clusters, each closing a specific measured gap, plus a
measure-and-lock task at the end. Tests follow the repo's existing style: integration tests under
`tests/` driving the real `bb` binary with `assert_cmd` and mocking HTTP with `wiremock`; inline
`#[cfg(test)]` unit tests only in files that already have them.

**Tech Stack:** Rust 2021, tokio, wiremock 0.6, assert_cmd, predicates, serial_test, tempfile,
cargo-llvm-cov.

## Global Constraints

- **No production code changes.** Not one line of behaviour in `src/` may change. The only edits
  permitted inside `src/` are additions to existing `#[cfg(test)] mod tests` blocks. A diff that
  modifies production code is a defect, even if tests pass.
- **No coverage exclusions.** Never add `#[coverage(off)]`, `#[cfg(not(coverage))]`,
  `--ignore-filename-regex`, or any other mechanism that removes code from the denominator.
- **Every test sets `BB_KEYRING_DISABLE=1`** unless it is specifically testing keyring behaviour.
  A test that touches the real OS keyring destroys the developer's stored token — this has already
  happened once in this repo's history.
- **No test may launch a browser.** Never invoke `bb browse` without `--print`.
- **`assert_cmd` inherits the parent environment.** Any test that relies on git-based repository
  resolution MUST call `.env_remove("BB_REPO")`, and any test that must not see ambient credentials
  MUST call `.env_remove("BB_EMAIL")` and `.env_remove("BB_TOKEN")`. Forgetting this produces tests
  that pass on one machine and fail on another.
- **Test files start with** `#![allow(clippy::unwrap_used)] // test code is exempt from the unwrap/expect ban`
  — clippy denies `unwrap_used` package-wide.
- **Local gates, all three green before the branch is done:** `cargo fmt --all --check`,
  `cargo clippy --all-targets -- -D warnings`, `cargo test`. CI additionally sets
  `RUSTFLAGS: -D warnings`.
- **Coverage is measured with:** `cargo llvm-cov --all-features --workspace --summary-only`, run with
  `BB_KEYRING_DISABLE=1` in the environment. `cargo-llvm-cov` counts `#[cfg(test)]` code in the
  denominator; that is expected and must not be worked around.
- Existing test count is the baseline: no existing test may be deleted, weakened, or renamed.

---

### Task 1: `auth` command coverage

Closes `commands/auth.rs` 51.89% → target ≥85%. Covers `auth.rs:51,58-59,65-68,79-84,86-89,92-97,99-110,124,146-151`.

`login` needs no TTY when both `--email` and `--token-stdin` are given: `would_prompt` at
`auth.rs:51` is false, so the non-interactive guard at `auth.rs:52` passes and no `inquire` prompt is
reached. This is the key that unlocks the whole login path.

**Files:**
- Modify: `tests/auth.rs` (append; existing tests stay untouched)

**Interfaces:**
- Consumes: nothing from other tasks.
- Produces: nothing other tasks depend on.

`Cargo.toml` already has `wiremock`, `tokio`, `tempfile`, `assert_cmd`, and `predicates` as dev
dependencies. `tests/auth.rs` currently has no `wiremock` usage, so add the imports.

- [ ] **Step 1: Write the failing tests**

Append to `tests/auth.rs`:

```rust
use tempfile::tempdir;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// `login` with --email and --token-stdin never prompts, so the whole verify-then-store
/// path runs without a tty.
#[tokio::test]
async fn login_verifies_the_token_and_reports_the_account() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/user"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"display_name": "Dev Person"})),
        )
        .mount(&server)
        .await;

    let out = Command::cargo_bin("bb")
        .unwrap()
        .args(["auth", "login", "--email", "dev@example.com", "--token-stdin"])
        .env("BB_API_BASE", server.uri())
        .env("BB_KEYRING_DISABLE", "1")
        .env("NO_COLOR", "1")
        .env_remove("BB_EMAIL")
        .env_remove("BB_TOKEN")
        .write_stdin("ATATT3xFfGF0abcd")
        .output()
        .unwrap();

    assert!(out.status.success(), "login failed: {out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("Dev Person"), "no account name: {stdout}");
    assert!(stdout.contains("****abcd"), "no redacted tail: {stdout}");
    assert!(
        !stdout.contains("ATATT3xFfGF0abcd"),
        "token leaked: {stdout}"
    );
}

/// The same path in --json mode must emit pure JSON on stdout and no human lines.
#[tokio::test]
async fn login_json_emits_pure_json() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/user"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"display_name": "Dev Person"})),
        )
        .mount(&server)
        .await;

    let out = Command::cargo_bin("bb")
        .unwrap()
        .args([
            "auth",
            "login",
            "--email",
            "dev@example.com",
            "--token-stdin",
            "--json",
        ])
        .env("BB_API_BASE", server.uri())
        .env("BB_KEYRING_DISABLE", "1")
        .env("NO_COLOR", "1")
        .env_remove("BB_EMAIL")
        .env_remove("BB_TOKEN")
        .write_stdin("ATATT3xFfGF0abcd")
        .output()
        .unwrap();

    assert!(out.status.success(), "login failed: {out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let value: serde_json::Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("stdout was not pure JSON: {e}\nstdout: {stdout}"));
    assert_eq!(value["account"], "Dev Person");
    assert_eq!(value["email"], "dev@example.com");
    assert_eq!(value["token"], "****abcd");
}

/// A value that is not an email address is rejected before any network call.
#[test]
fn login_rejects_an_email_without_an_at_sign() {
    Command::cargo_bin("bb")
        .unwrap()
        .args(["auth", "login", "--email", "not-an-email", "--token-stdin"])
        .env("BB_API_BASE", "http://127.0.0.1:1")
        .env("BB_KEYRING_DISABLE", "1")
        .env("NO_COLOR", "1")
        .env_remove("BB_EMAIL")
        .env_remove("BB_TOKEN")
        .write_stdin("some-token")
        .assert()
        .failure()
        .stderr(contains("atlassian account email"));
}

/// A leftover plaintext credential file from the PHP-era CLI must be called out on logout.
#[test]
fn logout_warns_about_a_legacy_plaintext_credential_file() {
    let home = tempdir().unwrap();
    std::fs::write(
        home.path().join(".bitbucket-rest-cli-config.json"),
        "{\"token\":\"whatever\"}",
    )
    .unwrap();

    Command::cargo_bin("bb")
        .unwrap()
        .args(["auth", "logout"])
        .env("HOME", home.path())
        .env("BB_KEYRING_DISABLE", "1")
        .env("NO_COLOR", "1")
        .assert()
        .success()
        .stderr(contains("legacy plaintext credential file"));
}

/// A failing identity check must not fail the command — the account is simply unknown.
#[tokio::test]
async fn status_reports_an_unverified_account_when_the_identity_check_fails() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/user"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let out = Command::cargo_bin("bb")
        .unwrap()
        .args(["auth", "status", "--json"])
        .env("BB_EMAIL", "dev@example.com")
        .env("BB_TOKEN", "ATATT3xFfGF0abcd")
        .env("BB_API_BASE", server.uri())
        .env("BB_KEYRING_DISABLE", "1")
        .output()
        .unwrap();

    assert!(out.status.success(), "status failed: {out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let value: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert!(
        value["account"].is_null(),
        "expected a null account, got {value}"
    );
}
```

- [ ] **Step 2: Run the tests to see them pass or fail for the right reason**

Run: `BB_KEYRING_DISABLE=1 cargo test --test auth`

These are new tests against existing behaviour, so they should pass immediately. If one fails, the
failure tells you something real about the code — read it and report it rather than reshaping the
assertion to match. Two specific things to check if `login` tests fail:
- If the process hangs, `would_prompt` logic differs from the analysis: report it, do not add a
  timeout hack.
- If `display_name` is not the field name in `api::models::User`, read `src/api/models.rs` and use
  the real serde field name.

- [ ] **Step 3: Verify the coverage moved**

Run: `BB_KEYRING_DISABLE=1 cargo llvm-cov --all-features --workspace --summary-only 2>&1 | grep -E 'auth.rs|TOTAL'`

Expected: `commands/auth.rs` rises from 51.89% to at least 85%. Record the actual TOTAL in your
report.

- [ ] **Step 4: Gates**

Run: `cargo fmt --all --check && cargo clippy --all-targets -- -D warnings && cargo test`

- [ ] **Step 5: Commit**

```bash
git add tests/auth.rs
git commit -m "test: cover the auth login, logout, and status paths"
```

---

### Task 2: `repo::resolve` and `git` coverage

Closes the largest untested unit in the crate: `repo::resolve()` (`repo.rs:85-121`) has no test at
all. Covering it also drives `git::current_branch`, `remote_url`, `remotes`, and `in_repo`
(`git.rs:34-55`).

**Critical constraint:** `repo::resolve()` reaches git through `git::in_repo()`,
`git::remote_url()`, and `git::remotes()`, all of which run `git` in the **process working
directory**. A unit test cannot change the process cwd safely (it is global state shared by parallel
tests). These must therefore be integration tests using `assert_cmd`'s `.current_dir()`.

`bb browse --print` is the right driver: it calls `repo::resolve()` and then only prints, making no
network request and launching no browser.

**Files:**
- Create: `tests/repo_resolve.rs`
- Test: the same file

**Interfaces:**
- Consumes: nothing from other tasks.
- Produces: nothing other tasks depend on.

- [ ] **Step 1: Write the failing tests**

Create `tests/repo_resolve.rs`:

```rust
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

#[test]
fn a_whitespace_only_bb_repo_falls_through_to_git() {
    let repo = git_repo();
    run_git(
        repo.path(),
        &["remote", "add", "origin", "git@bitbucket.org:acme/widgets.git"],
    );

    let mut cmd = Command::cargo_bin("bb").unwrap();
    cmd.current_dir(repo.path())
        .args(["browse", "--print"])
        .env("BB_KEYRING_DISABLE", "1")
        .env("NO_COLOR", "1")
        .env("BB_REPO", "   ")
        .assert()
        .success()
        .stdout(contains("https://bitbucket.org/acme/widgets"));
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
        &["remote", "add", "origin", "https://github.com/someone/thing.git"],
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
        &["remote", "add", "origin", "https://github.com/someone/fork.git"],
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
#[test]
fn a_detached_head_is_reported_when_the_source_branch_is_inferred() {
    let repo = git_repo();
    run_git(
        repo.path(),
        &["remote", "add", "origin", "git@bitbucket.org:acme/widgets.git"],
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
        .stderr(contains("detached HEAD"));
}
```

- [ ] **Step 2: Add the `remote_url` unit test**

`git::remote_url`'s empty-url branch (`git.rs:46-48`) needs a remote configured with an empty url,
which `git remote add` will not create. Set it directly through `git config`. This one can be an
integration test in the same file, since it goes through `resolve`:

```rust
/// A remote whose url is configured empty must be reported, not silently treated as a
/// repository named "".
#[test]
fn a_remote_with_an_empty_url_is_skipped_rather_than_parsed() {
    let repo = git_repo();
    run_git(repo.path(), &["config", "remote.origin.url", ""]);
    run_git(
        repo.path(),
        &["remote", "add", "backup", "git@bitbucket.org:acme/widgets.git"],
    );

    // The empty `origin` url yields an error from `remote_url`, which `resolve` skips
    // with `if let Ok(...)`, so the good remote still wins.
    bb_in(repo.path())
        .assert()
        .success()
        .stdout(contains("https://bitbucket.org/acme/widgets"));
}
```

- [ ] **Step 3: Run the tests**

Run: `BB_KEYRING_DISABLE=1 cargo test --test repo_resolve`

If `git init` produces a warning about the default branch name on the CI image, that is noise on
stderr and does not fail the test. If `a_remote_with_an_empty_url_is_skipped_rather_than_parsed`
fails, read what `git config remote.origin.url ""` actually stores and adjust the setup — but do not
weaken the assertion about the good remote winning.

- [ ] **Step 4: Verify the coverage moved**

Run: `BB_KEYRING_DISABLE=1 cargo llvm-cov --all-features --workspace --summary-only 2>&1 | grep -E 'repo.rs|git.rs|TOTAL'`

Expected: `repo.rs` rises from 78.57% to ≥95%, `git.rs` from 72.73% to ≥90%.

- [ ] **Step 5: Gates**

Run: `cargo fmt --all --check && cargo clippy --all-targets -- -D warnings && cargo test`

- [ ] **Step 6: Commit**

```bash
git add tests/repo_resolve.rs
git commit -m "test: cover repository resolution from git remotes"
```

---

### Task 3: `api::Client` remaining branches

Covers `api/mod.rs:85-88` (403), `92-95` (429), `153-166` (`put_json`), and `180-181` (the
`MAX_PAGES` cap).

**Files:**
- Modify: `tests/api_client.rs` (append)

**Interfaces:**
- Consumes: `support::client_for(&str) -> Client`, already defined in `tests/support/mod.rs`.
- Produces: nothing other tasks depend on.

- [ ] **Step 1: Write the failing tests**

Append to `tests/api_client.rs`:

```rust
#[tokio::test]
async fn maps_403_to_a_scope_hint() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/forbidden"))
        .respond_with(ResponseTemplate::new(403))
        .mount(&server)
        .await;

    let err = client_for(&server.uri())
        .get_json::<Item>("/forbidden")
        .await
        .unwrap_err();
    match err {
        BbError::Api { status, message } => {
            assert_eq!(status, 403);
            // The message must point at the likely cause rather than echo the body.
            assert!(message.contains("scope"), "unhelpful message: {message}");
        }
        other => panic!("unexpected: {other:?}"),
    }
}

#[tokio::test]
async fn maps_429_to_a_rate_limit_message() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/limited"))
        .respond_with(ResponseTemplate::new(429))
        .mount(&server)
        .await;

    let err = client_for(&server.uri())
        .get_json::<Item>("/limited")
        .await
        .unwrap_err();
    match err {
        BbError::Api { status, message } => {
            assert_eq!(status, 429);
            assert!(
                message.contains("rate limited"),
                "unhelpful message: {message}"
            );
        }
        other => panic!("unexpected: {other:?}"),
    }
}

#[tokio::test]
async fn put_json_sends_the_body_and_parses_the_response() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/thing/1"))
        .and(body_json(serde_json::json!({"content": "edited"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"id": 42})))
        .mount(&server)
        .await;

    let item: Item = client_for(&server.uri())
        .put_json("/thing/1", &serde_json::json!({"content": "edited"}))
        .await
        .unwrap();
    assert_eq!(item.id, 42);
}

/// A server that keeps handing out fresh `next` links must not be followed forever.
#[tokio::test]
async fn paginate_stops_at_the_page_cap() {
    let server = MockServer::start().await;
    let base = server.uri();

    // Page N links to page N+1, each a distinct url so the repeat-detection guard
    // does not fire — only the hard page cap can stop this.
    for n in 0..150u32 {
        let next = format!("{base}/pages?page={}", n + 1);
        Mock::given(method("GET"))
            .and(path("/pages"))
            .and(query_param("page", n.to_string()))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "values": [{"id": n}],
                "next": next
            })))
            .mount(&server)
            .await;
    }

    let items: Vec<Item> = client_for(&server.uri())
        .paginate("/pages?page=0")
        .await
        .unwrap();
    // MAX_PAGES is 100 in src/api/mod.rs.
    assert_eq!(items.len(), 100, "expected the page cap to stop pagination");
}
```

Extend the existing `wiremock::matchers` import line in this file to include `body_json`:

```rust
use wiremock::matchers::{body_json, header, method, path, query_param};
```

- [ ] **Step 2: Run the tests**

Run: `cargo test --test api_client`

If `paginate_stops_at_the_page_cap` is slow (150 mounted mocks), that is acceptable as long as it
finishes in a few seconds. If it exceeds ~30s, reduce the loop to `0..105` — the cap is 100, so 105
pages still proves the point. Do not change `MAX_PAGES` in `src/`.

- [ ] **Step 3: Verify the coverage moved**

Run: `BB_KEYRING_DISABLE=1 cargo llvm-cov --all-features --workspace --summary-only 2>&1 | grep -E 'api/mod.rs|TOTAL'`

Expected: `api/mod.rs` rises from 83.56% to ≥95%.

- [ ] **Step 4: Gates**

Run: `cargo fmt --all --check && cargo clippy --all-targets -- -D warnings && cargo test`

- [ ] **Step 5: Commit**

```bash
git add tests/api_client.rs
git commit -m "test: cover the 403, 429, put_json, and page-cap paths"
```

---

### Task 4: `pr create` validation and comment-rendering edge cases

Covers `pr.rs:276-277` (empty target), `pr.rs:279-281` (source equals target), and
`pr_comments.rs:113,129,134,135` — the "none" branches when a comment list is empty, and the two
inline-location fallbacks when a comment has a file but no line, or neither.

`output::heading` is already covered; do not add a test for it. `output.rs`'s remaining gap is
`warn` (covered by Task 1) and the spinner body at `output.rs:116-120`, which runs only when stderr
is a terminal and is therefore unreachable from the test harness.

**Files:**
- Modify: `tests/pr_create.rs` (append the two validation tests)
- Modify: `tests/pr_view.rs` (append the three rendering tests and one mock helper)

**Interfaces:**
- Consumes: whatever `bb()`-style helper already exists in each file — read the top of each file and
  reuse its helper rather than defining a second one.
- Produces: nothing other tasks depend on.

- [ ] **Step 1: Read the existing helpers**

Read `tests/pr_create.rs` and `tests/pr_view.rs` first. Both define `fn bb(server: &MockServer) ->
Command`, which sets `BB_EMAIL`, `BB_TOKEN`, `BB_API_BASE`, `BB_REPO`, and `NO_COLOR`. Reuse it. Do
not introduce a competing helper with the same purpose.

- [ ] **Step 2: Write the two validation tests**

Append to `tests/pr_create.rs`. Both fail before any HTTP request is made, but the file's `bb` helper
takes a `MockServer`, so start one and mount nothing — a request would then fail loudly, which is
what you want, since these tests assert the code never gets that far:

```rust
/// A target of only separators leaves no branch to target.
#[tokio::test]
async fn create_rejects_a_target_that_is_only_commas() {
    let server = MockServer::start().await;
    bb(&server)
        .args(["pr", "create", ",,", "--source", "feature/x"])
        .assert()
        .failure()
        .stderr(contains("no target branch given"));
}

/// Opening a pull request from a branch onto itself is always a mistake.
#[tokio::test]
async fn create_rejects_a_source_equal_to_the_target() {
    let server = MockServer::start().await;
    bb(&server)
        .args(["pr", "create", "main", "--source", "main"])
        .assert()
        .failure()
        .stderr(contains("source and target are both `main`"));
}
```

Note the first test uses `,,` rather than an empty string: `target` is a positional argument, and an
empty positional may be rejected by clap before the code under test runs. If `,,` does not reach the
check either, read how `target` is declared in `src/commands/pr.rs` and pick a value that does.

- [ ] **Step 3: Write the comment-rendering tests**

These go in `tests/pr_view.rs`, not `tests/pr_comment.rs` — that is the file that drives
`bb pr view`. Read its existing `mock_pr_and_comments` helper (around line 52) for the exact comment
JSON shape, then append these three tests, mirroring that shape:

```rust
/// With no comments at all, both sections must say so rather than render empty.
#[tokio::test]
async fn view_reports_none_for_empty_comment_sections() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repositories/acme/widgets/pullrequests/7"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": 7,
            "title": "A change",
            "state": "OPEN",
            "author": { "display_name": "Me" },
            "source": { "branch": { "name": "feature/x" } },
            "destination": { "branch": { "name": "main" } },
            "links": { "html": { "href": "https://bitbucket.org/acme/widgets/pull-requests/7" } }
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repositories/acme/widgets/pullrequests/7/comments"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "values": []
        })))
        .mount(&server)
        .await;

    let out = bb(&server).args(["pr", "view", "7"]).output().unwrap();
    assert!(out.status.success(), "view failed: {out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    // One "none" under general comments, one under inline comments.
    assert_eq!(
        stdout.matches("none").count(),
        2,
        "expected both sections to report none: {stdout}"
    );
}

/// An inline comment carrying a file but no line renders the bare path.
#[tokio::test]
async fn an_inline_comment_without_a_line_renders_just_the_path() {
    let server = MockServer::start().await;
    mock_pr_and_comments_with(
        &server,
        serde_json::json!({
            "values": [{
                "id": 500,
                "content": { "raw": "whole-file note" },
                "user": { "display_name": "Reviewer" },
                "created_on": "2026-08-04T10:00:00+00:00",
                "inline": { "path": "src/lib.rs" }
            }]
        }),
    )
    .await;

    bb(&server)
        .args(["pr", "view", "7"])
        .assert()
        .success()
        .stdout(contains("src/lib.rs"))
        .stdout(contains("whole-file note"));
}

/// An inline comment with neither path nor line falls back to a dash rather than
/// rendering an empty location.
#[tokio::test]
async fn an_inline_comment_without_a_location_renders_a_dash() {
    let server = MockServer::start().await;
    mock_pr_and_comments_with(
        &server,
        serde_json::json!({
            "values": [{
                "id": 501,
                "content": { "raw": "location-less note" },
                "user": { "display_name": "Reviewer" },
                "created_on": "2026-08-04T10:00:00+00:00",
                "inline": {}
            }]
        }),
    )
    .await;

    bb(&server)
        .args(["pr", "view", "7"])
        .assert()
        .success()
        .stdout(contains("location-less note"))
        .stdout(contains("-  (comment 501)"));
}
```

Add this helper to `tests/pr_view.rs` alongside the existing `mock_pr_and_comments`, so the two
tests above can vary the comment payload. Reuse the pull-request JSON from the existing helper rather
than duplicating a third copy — read it and factor the shared part if it is already a separate
function:

```rust
/// Same pull-request mock as `mock_pr_and_comments`, with a caller-supplied comment page.
async fn mock_pr_and_comments_with(server: &MockServer, comments: serde_json::Value) {
    Mock::given(method("GET"))
        .and(path("/repositories/acme/widgets/pullrequests/7"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": 7,
            "title": "A change",
            "state": "OPEN",
            "author": { "display_name": "Me" },
            "source": { "branch": { "name": "feature/x" } },
            "destination": { "branch": { "name": "main" } },
            "links": { "html": { "href": "https://bitbucket.org/acme/widgets/pull-requests/7" } }
        })))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repositories/acme/widgets/pullrequests/7/comments"))
        .respond_with(ResponseTemplate::new(200).set_body_json(comments))
        .mount(server)
        .await;
}
```

If `tests/pr_view.rs`'s command helper is not named `bb`, use whatever it is actually called. If the
`inline` marker or spacing in the expected `"-  (comment 501)"` string does not match what
`pr_comments.rs:138` prints, read that line and match it exactly — the double space there is
deliberate.

- [ ] **Step 4: Run the tests**

Run: `BB_KEYRING_DISABLE=1 cargo test --test pr_create --test pr_view`

- [ ] **Step 5: Verify the coverage moved**

Run: `BB_KEYRING_DISABLE=1 cargo llvm-cov --all-features --workspace --summary-only 2>&1 | grep -E 'pr.rs|pr_comments.rs|TOTAL'`

Expected: `commands/pr.rs` ≥92%, `commands/pr_comments.rs` ≥96%.

- [ ] **Step 6: Gates**

Run: `cargo fmt --all --check && cargo clippy --all-targets -- -D warnings && cargo test`

- [ ] **Step 7: Commit**

```bash
git add tests/pr_create.rs tests/pr_view.rs
git commit -m "test: cover pr create validation and comment rendering fallbacks"
```

---

### Task 5: Measure, and close any remaining gap

The four preceding tasks are estimated at ~113 lines against a need of ~110. This task establishes
whether that held, and closes the gap if it did not.

**Files:**
- Modify: `src/credentials.rs` (inline `#[cfg(test)] mod tests` only — **contingency, only if
  below 90%**)
- Create: `codecov.yml`

**Interfaces:**
- Consumes: the coverage state produced by Tasks 1-4.
- Produces: the final measured number, reported in your task report.

- [ ] **Step 1: Measure the full picture**

Run: `BB_KEYRING_DISABLE=1 cargo llvm-cov --all-features --workspace --summary-only`

Record the TOTAL line coverage. Then two cases.

- [ ] **Step 2 (only if TOTAL ≥ 90%): skip to step 4**

State the measured number in your report and move on. Do not add tests the goal does not need.

- [ ] **Step 3 (only if TOTAL < 90%): add the keyring contingency tests**

`credentials::load` (`credentials.rs:86-94`) and `store` (`102-112`) never run their real-keyring
arms because every test sets `BB_KEYRING_DISABLE=1`. `keyring` 3 ships a mock credential store that
can be installed process-globally, which reaches those arms without touching the real OS keyring.

`src/credentials.rs` already has a `#[cfg(test)]` module that installs a custom credential builder
and marks the test `#[serial]` — read it first (the `PanicOnConstruction` test) and follow that exact
pattern, including its comment about process-global mutation.

Add to that module:

```rust
#[test]
#[serial]
fn store_then_load_round_trips_through_the_keyring() {
    // **IMPORTANT: Global State Mutation (process-wide, no cleanup)**
    // Installs keyring's mock store as the process default, exactly as the
    // PanicOnConstruction test above does. Marked #[serial] for that reason.
    keyring::set_default_credential_builder(keyring::mock::default_credential_builder());

    // The env guard must be absent for the real-keyring arms to run at all.
    std::env::remove_var("BB_KEYRING_DISABLE");
    std::env::remove_var("BB_EMAIL");
    std::env::remove_var("BB_TOKEN");

    store("dev@example.com", &SecretString::from("t0ken-value".to_string())).unwrap();

    // keyring's mock gives each Entry::new independent storage
    // (CredentialPersistence::EntryOnly), so load() cannot see what store() wrote.
    // What this test proves is that both functions run their real-keyring arms
    // without erroring — which is exactly the uncovered code.
    let _ = load();

    delete().unwrap();
}
```

Note honestly in the test's comment what it does and does not prove — the mock's per-entry storage
means this is a smoke test of the keyring arms, not a true round-trip. Do not write a comment
claiming it verifies persistence.

Then re-measure. If still below 90%, report the number and the remaining largest gaps rather than
inventing low-value tests; the controller decides what to do next.

- [ ] **Step 4: Set the Codecov target**

Create `codecov.yml` at the repository root:

```yaml
# Coverage is advisory: it reports on pull requests but never blocks a merge, so a
# legitimate change is not held up by an arithmetic cliff.
coverage:
  status:
    project:
      default:
        target: 90%
        threshold: 1%
        informational: true
    patch:
      default:
        informational: true
comment:
  layout: "reach, diff, flags, files"
  require_changes: true
```

- [ ] **Step 5: Update the README coverage claim if it states a number**

Run: `rg -n '85%|84\.|coverage' README.md`

If the README states a specific coverage percentage or test count, update it to the measured values.
If it only carries the Codecov badge, change nothing — the badge updates itself.

- [ ] **Step 6: Gates**

Run: `cargo fmt --all --check && cargo clippy --all-targets -- -D warnings && cargo test`

- [ ] **Step 7: Commit**

```bash
git add -- codecov.yml src/credentials.rs README.md
git commit -m "test: set the coverage target to 90% and close the remaining gap"
```

Note: `git add -- <paths>` with explicit paths, never `git add -A` — this repo has a history of a
bulk add staging files that were meant to stay out.

---

## Verification (whole branch)

1. `BB_KEYRING_DISABLE=1 cargo llvm-cov --all-features --workspace --summary-only` reports TOTAL
   line coverage ≥ 90%.
2. `cargo fmt --all --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test` all green.
3. `git diff --stat main...HEAD` touches only `tests/`, `#[cfg(test)]` blocks inside `src/`,
   `codecov.yml`, `README.md`, and `docs/`. **Any production-code change in `src/` is a defect.**
4. No existing test was deleted, renamed, or weakened.
