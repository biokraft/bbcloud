# PR reviewers, review state and list filters — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make reviewer identity and review state first-class in `bb` — visible in `pr list`, mutable through `bb pr reviewers add/remove`, and filterable, including "which pull requests are waiting on me".

**Architecture:** Reviewer state is derived in `src/api/models.rs` from Bitbucket's `participants[]` unioned with `reviewers[]`. `bb pr list` must ask for those fields explicitly (`fields=%2Bvalues…`) because the paginated endpoint omits them — that omission is why today's `REVIEWERS` column is always empty. Human-name-to-uuid lookup lives in one new module, `src/users.rs`, used by both the reviewer mutations and the list filters. `pr list` moves out of the already-long `pr.rs` into `src/commands/pr_list.rs`; reviewer mutations go in `src/commands/pr_reviewers.rs`.

**Tech Stack:** Rust 2021, crate `bbcloud` / lib `bb_cli` / bin `bb`, clap derive, reqwest, serde, comfy-table, `wiremock` + `tokio` + `assert_cmd` + `predicates` for tests.

## Global Constraints

- Spec: `docs/superpowers/specs/2026-08-11-pr-reviewers-design.md`. Where this plan and the spec disagree, stop and ask.
- **Every test that runs the `bb` binary MUST set `BB_KEYRING_DISABLE=1`.** A test that reaches the real OS keyring destroys the developer's stored API token. This has happened in this project before.
- Never `git add -A`. Always an explicit path list.
- Approve, merge, decline and resolve-thread stay unsupported. Do not add them, do not mention them in help text.
- `--json` output must be pure JSON on every path, including zero rows. `output::print_table` prints "nothing to show" and is not format-aware, so every call site must gate on `ctx.format`.
- No `unwrap`/`expect` in `src/` (clippy denies it). Test code is exempt via the existing `#![allow(clippy::unwrap_used)]` header.
- Gate before every commit: `cargo fmt --all`, then `cargo clippy --all-targets -- -D warnings`, then `cargo test`. All three clean.
- Work in the worktree `.worktrees/pr-reviewers` on branch `feat/pr-reviewers`.
- Review-state marks, exact glyphs: approved `✓`, changes requested `✗`, no state `·`.
- Reviewer JSON shape, exactly: `{"name": String, "uuid": String|null, "state": "approved"|"changes_requested"|"pending"}`.

---

## File Structure

| file | responsibility |
|---|---|
| `src/api/models.rs` (modify) | `draft`, participant `role`/`approved`, `User::name()`, `ReviewState`, `ReviewerState`, `PullRequest::reviewer_states()`, `PullRequest::display_state()`. Pure data + derivation, no I/O. |
| `src/users.rs` (new) | `resolve_user` — one human-typed name to one `User`, or a helpful error. |
| `src/commands/pr_list.rs` (new) | `bb pr list`: fetch, filter, render. |
| `src/commands/pr_reviewers.rs` (new) | `bb pr reviewers` list/add/remove. |
| `src/commands/pr.rs` (modify) | loses `list`; keeps `Ctx`, `create`, `diff`, `files`, `commits`. |
| `src/lib.rs`, `src/commands/mod.rs` (modify) | register `users`, `pr_list`, `pr_reviewers`. |
| `src/main.rs` (modify) | new flags on `List`, new `Reviewers` subcommand. |
| `tests/pr_list.rs`, `tests/pr_reviewers.rs`, `tests/user_resolve.rs` (new) | one test file per new unit. |
| `README.md` (modify) | document the new commands and flags. |

---

## Task 1: Review state in the data layer

**Files:**
- Modify: `src/api/models.rs:3-49` (`User`, `Participant`, `PullRequest`)
- Test: `src/api/models.rs` — new `#[cfg(test)] mod tests` at the end of the file

Pure derivation logic with no I/O, so it is unit-tested inline. `src/git.rs`, `src/repo.rs` and `src/output.rs` already carry inline `#[cfg(test)]` modules; follow that pattern.

**Interfaces:**
- Consumes: nothing.
- Produces:
  - `pub enum ReviewState { Approved, ChangesRequested, Pending }` with `ReviewState::from_api(Option<&str>) -> ReviewState` and `ReviewState::mark(self) -> &'static str`. Serializes as `"approved"` / `"changes_requested"` / `"pending"`.
  - `pub struct ReviewerState { pub name: String, pub uuid: Option<String>, pub state: ReviewState }`, `Debug + Clone + Serialize`.
  - `User::name(&self) -> &str` — `display_name`, else `nickname`, else `"-"`.
  - `User.account_id: Option<String>`, `Participant.role: Option<String>`, `Participant.approved: bool`, `PullRequest.draft: bool`.
  - `PullRequest::reviewer_states(&self) -> Vec<ReviewerState>`
  - `PullRequest::display_state(&self) -> String`

- [ ] **Step 1: Write the failing tests**

Append to `src/api/models.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn pr_from(json: serde_json::Value) -> PullRequest {
        serde_json::from_value(json).expect("fixture should deserialize")
    }

    #[test]
    fn reviewer_states_reads_state_from_participants() {
        let pr = pr_from(serde_json::json!({
            "id": 1,
            "reviewers": [
                { "uuid": "{a}", "display_name": "Ana" },
                { "uuid": "{b}", "display_name": "Bo" },
                { "uuid": "{c}", "display_name": "Cy" }
            ],
            "participants": [
                { "role": "REVIEWER", "state": "approved", "user": { "uuid": "{a}", "display_name": "Ana" } },
                { "role": "REVIEWER", "state": "changes_requested", "user": { "uuid": "{b}", "display_name": "Bo" } },
                { "role": "REVIEWER", "state": null, "user": { "uuid": "{c}", "display_name": "Cy" } }
            ]
        }));

        let states = pr.reviewer_states();
        assert_eq!(states.len(), 3);
        assert_eq!(states[0].name, "Ana");
        assert_eq!(states[0].state, ReviewState::Approved);
        assert_eq!(states[1].state, ReviewState::ChangesRequested);
        assert_eq!(states[2].state, ReviewState::Pending);
    }

    /// A tagged reviewer who has not opened the pull request at all is absent from
    /// `participants`. They must still be listed, or the column under-reports who is
    /// on the hook.
    #[test]
    fn reviewer_states_includes_a_reviewer_missing_from_participants() {
        let pr = pr_from(serde_json::json!({
            "id": 1,
            "reviewers": [{ "uuid": "{a}", "display_name": "Ana" }],
            "participants": []
        }));

        let states = pr.reviewer_states();
        assert_eq!(states.len(), 1);
        assert_eq!(states[0].name, "Ana");
        assert_eq!(states[0].state, ReviewState::Pending);
    }

    /// Someone who merely commented has role PARTICIPANT. Counting them as a
    /// reviewer would invent reviewers nobody tagged.
    #[test]
    fn reviewer_states_excludes_plain_participants() {
        let pr = pr_from(serde_json::json!({
            "id": 1,
            "reviewers": [],
            "participants": [
                { "role": "PARTICIPANT", "state": "approved", "user": { "uuid": "{z}", "display_name": "Zed" } }
            ]
        }));

        assert!(pr.reviewer_states().is_empty());
    }

    /// The same person appears in both arrays; they must be listed once, with the
    /// participant state rather than a duplicate Pending row.
    #[test]
    fn reviewer_states_does_not_duplicate_across_both_arrays() {
        let pr = pr_from(serde_json::json!({
            "id": 1,
            "reviewers": [{ "uuid": "{a}", "display_name": "Ana" }],
            "participants": [
                { "role": "REVIEWER", "state": "approved", "user": { "uuid": "{a}", "display_name": "Ana" } }
            ]
        }));

        let states = pr.reviewer_states();
        assert_eq!(states.len(), 1);
        assert_eq!(states[0].state, ReviewState::Approved);
    }

    /// Dedup must survive a missing uuid on one side by falling back to the name.
    #[test]
    fn reviewer_states_dedups_by_name_when_a_uuid_is_absent() {
        let pr = pr_from(serde_json::json!({
            "id": 1,
            "reviewers": [{ "display_name": "Ana" }],
            "participants": [
                { "role": "REVIEWER", "state": "approved", "user": { "display_name": "Ana" } }
            ]
        }));

        assert_eq!(pr.reviewer_states().len(), 1);
    }

    #[test]
    fn marks_are_stable_glyphs() {
        assert_eq!(ReviewState::Approved.mark(), "✓");
        assert_eq!(ReviewState::ChangesRequested.mark(), "✗");
        assert_eq!(ReviewState::Pending.mark(), "·");
    }

    #[test]
    fn review_state_serializes_in_snake_case() {
        let json = serde_json::to_string(&ReviewState::ChangesRequested).unwrap();
        assert_eq!(json, "\"changes_requested\"");
    }

    #[test]
    fn draft_wins_over_open_state() {
        let pr = pr_from(serde_json::json!({ "id": 1, "state": "OPEN", "draft": true }));
        assert_eq!(pr.display_state(), "Draft");
    }

    #[test]
    fn display_state_title_cases_the_api_value() {
        let pr = pr_from(serde_json::json!({ "id": 1, "state": "DECLINED" }));
        assert_eq!(pr.display_state(), "Declined");
    }

    #[test]
    fn display_state_without_a_state_is_a_dash() {
        let pr = pr_from(serde_json::json!({ "id": 1 }));
        assert_eq!(pr.display_state(), "-");
    }

    #[test]
    fn user_name_prefers_display_name_then_nickname() {
        let full: User =
            serde_json::from_value(serde_json::json!({ "display_name": "Ana Cruz", "nickname": "ana" }))
                .unwrap();
        assert_eq!(full.name(), "Ana Cruz");

        let nick_only: User = serde_json::from_value(serde_json::json!({ "nickname": "ana" })).unwrap();
        assert_eq!(nick_only.name(), "ana");

        let empty: User = serde_json::from_value(serde_json::json!({})).unwrap();
        assert_eq!(empty.name(), "-");
    }
}
```

Note: these fixtures deserialize a `PullRequest` from a partial object. That already works because every field except `id` is `Option` or `#[serde(default)]`; keep it that way.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib models`
Expected: FAIL to compile — `ReviewState` not found, no method `reviewer_states`, no field `draft`.

- [ ] **Step 3: Implement**

In `src/api/models.rs`, replace the `User`, `Participant` and `PullRequest` definitions and add the new types:

```rust
#[derive(Debug, Deserialize)]
pub struct User {
    pub uuid: Option<String>,
    pub account_id: Option<String>,
    pub display_name: Option<String>,
    pub nickname: Option<String>,
}

impl User {
    /// The name a human recognizes. `display_name` is what the Bitbucket web ui
    /// shows, so it is preferred over the nickname.
    pub fn name(&self) -> &str {
        self.display_name
            .as_deref()
            .or(self.nickname.as_deref())
            .unwrap_or("-")
    }
}

#[derive(Debug, Deserialize)]
pub struct Participant {
    pub user: Option<User>,
    pub state: Option<String>,
    pub role: Option<String>,
    #[serde(default)]
    pub approved: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewState {
    Approved,
    ChangesRequested,
    Pending,
}

impl ReviewState {
    pub fn from_api(state: Option<&str>) -> Self {
        match state {
            Some("approved") => Self::Approved,
            Some("changes_requested") => Self::ChangesRequested,
            _ => Self::Pending,
        }
    }

    pub fn mark(self) -> &'static str {
        match self {
            Self::Approved => "✓",
            Self::ChangesRequested => "✗",
            Self::Pending => "·",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ReviewerState {
    pub name: String,
    pub uuid: Option<String>,
    pub state: ReviewState,
}
```

Add `#[serde(default)] pub draft: bool,` to `PullRequest`, then add to `impl PullRequest`:

```rust
    /// Who is on the hook for this pull request, and what each has decided.
    ///
    /// `reviewers[]` is the tagged set but carries no decision; `participants[]`
    /// carries the decision but also includes people who only commented. So
    /// participants with the REVIEWER role are the primary source, and anyone
    /// tagged who has not shown up there yet is appended as Pending.
    pub fn reviewer_states(&self) -> Vec<ReviewerState> {
        let mut out: Vec<ReviewerState> = self
            .participants
            .iter()
            .filter(|p| p.role.as_deref() == Some("REVIEWER"))
            .filter_map(|p| {
                p.user.as_ref().map(|u| ReviewerState {
                    name: u.name().to_string(),
                    uuid: u.uuid.clone(),
                    state: ReviewState::from_api(p.state.as_deref()),
                })
            })
            .collect();

        for reviewer in &self.reviewers {
            let already = out.iter().any(|seen| {
                match (seen.uuid.as_deref(), reviewer.uuid.as_deref()) {
                    (Some(a), Some(b)) => a == b,
                    // One side has no uuid, so the name is all there is to match on.
                    _ => seen.name == reviewer.name(),
                }
            });
            if !already {
                out.push(ReviewerState {
                    name: reviewer.name().to_string(),
                    uuid: reviewer.uuid.clone(),
                    state: ReviewState::Pending,
                });
            }
        }

        out
    }

    /// Bitbucket keeps `draft` as a boolean while `state` stays OPEN, so the two
    /// have to be folded into one word for the table.
    pub fn display_state(&self) -> String {
        if self.draft {
            return "Draft".to_string();
        }
        match self.state.as_deref() {
            Some(state) if !state.is_empty() => {
                let lower = state.to_lowercase();
                let mut chars = lower.chars();
                match chars.next() {
                    Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                    None => "-".to_string(),
                }
            }
            _ => "-".to_string(),
        }
    }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib models`
Expected: PASS, 11 tests.

- [ ] **Step 5: Confirm nothing else broke**

Run: `cargo fmt --all && cargo clippy --all-targets -- -D warnings && cargo test`
Expected: all green. `src/commands/pr.rs:52-62` still compiles — it reads `pr.reviewers` and `pr.participants`, both untouched in shape.

- [ ] **Step 6: Commit**

```bash
git add src/api/models.rs
git commit -m "feat(api): derive per-reviewer review state and draft state"
```

---

## Task 2: `pr list` moves out, asks for the fields, and shows state

**Files:**
- Create: `src/commands/pr_list.rs`
- Modify: `src/commands/pr.rs` — delete `PrRow`, `to_row` and `list` (lines 33-118)
- Modify: `src/commands/mod.rs` — add `pub mod pr_list;`
- Modify: `src/main.rs:198` — call `commands::pr_list::list`
- Modify: `tests/pr.rs:8-16` — add the missing `BB_KEYRING_DISABLE`
- Test: `tests/pr_list.rs` (new)

This task fixes the actual defect: the paginated `/pullrequests` endpoint omits `reviewers`, `participants` and `draft`, so the existing columns are always empty. Filters come in Task 5; this task delivers correct display.

**Interfaces:**
- Consumes: `ReviewState`, `ReviewerState`, `PullRequest::reviewer_states()`, `PullRequest::display_state()` from Task 1. `pr::Ctx` with `Ctx::path(&self, suffix: &str) -> String` and `ctx.format`.
- Produces: `pub async fn list(ctx: &Ctx, destination: Option<String>, state: String) -> Result<()>` in `commands::pr_list`. Task 5 replaces this signature with a `ListArgs` struct.

- [ ] **Step 1: Write the failing tests**

Create `tests/pr_list.rs`:

```rust
#![allow(clippy::unwrap_used)] // test code is exempt from the unwrap/expect ban

use assert_cmd::Command;
use predicates::str::contains;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn bb(server: &MockServer) -> Command {
    let mut cmd = Command::cargo_bin("bb").unwrap();
    cmd.env("BB_EMAIL", "dev@example.com")
        .env("BB_TOKEN", "t0ken-value")
        .env("BB_API_BASE", server.uri())
        .env("BB_REPO", "acme/widgets")
        .env("NO_COLOR", "1")
        // Without this a test can reach the real OS keyring and destroy the
        // developer's stored token.
        .env("BB_KEYRING_DISABLE", "1");
    cmd
}

/// A pull request with three reviewers in three different states.
fn pr_with_reviewers() -> serde_json::Value {
    serde_json::json!({
        "id": 7,
        "title": "fix the thing",
        "state": "OPEN",
        "author": { "nickname": "sean", "display_name": "Sean B" },
        "source": { "branch": { "name": "feature/a" } },
        "destination": { "branch": { "name": "main" } },
        "links": { "html": { "href": "https://bitbucket.org/acme/widgets/pull-requests/7" } },
        "reviewers": [
            { "uuid": "{a}", "display_name": "Ana" },
            { "uuid": "{b}", "display_name": "Bo" },
            { "uuid": "{c}", "display_name": "Cy" },
            { "uuid": "{d}", "display_name": "Dee" }
        ],
        "participants": [
            { "role": "REVIEWER", "state": "approved", "user": { "uuid": "{a}", "display_name": "Ana" } },
            { "role": "REVIEWER", "state": "changes_requested", "user": { "uuid": "{b}", "display_name": "Bo" } },
            { "role": "REVIEWER", "state": null, "user": { "uuid": "{c}", "display_name": "Cy" } },
            { "role": "PARTICIPANT", "state": "approved", "user": { "uuid": "{z}", "display_name": "Zed" } }
        ]
    })
}

async fn mount_list(server: &MockServer, values: serde_json::Value) {
    Mock::given(method("GET"))
        .and(path("/repositories/acme/widgets/pullrequests"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "values": values })))
        .mount(server)
        .await;
}

/// The whole point of the feature: without this parameter Bitbucket returns a
/// reduced pull-request object with no reviewers, participants or draft flag, and
/// the reviewer column is silently empty.
#[tokio::test]
async fn list_requests_the_reviewer_fields() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repositories/acme/widgets/pullrequests"))
        .and(query_param(
            "fields",
            "+values.reviewers,+values.participants,+values.draft",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "values": [] })))
        .mount(&server)
        .await;

    bb(&server).args(["pr", "list"]).assert().success();
}

#[tokio::test]
async fn list_marks_each_reviewer_with_their_state() {
    let server = MockServer::start().await;
    mount_list(&server, serde_json::json!([pr_with_reviewers()])).await;

    let out = bb(&server).args(["pr", "list"]).output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("Ana ✓"), "{text}");
    assert!(text.contains("Bo ✗"), "{text}");
    assert!(text.contains("Cy ·"), "{text}");
    // Tagged but never seen by the api's participant list.
    assert!(text.contains("Dee ·"), "{text}");
    // A commenter is not a reviewer.
    assert!(!text.contains("Zed"), "plain participant rendered as reviewer: {text}");
}

#[tokio::test]
async fn list_shows_the_pr_state_column() {
    let server = MockServer::start().await;
    let mut draft = pr_with_reviewers();
    draft["draft"] = serde_json::json!(true);
    let mut declined = pr_with_reviewers();
    declined["id"] = serde_json::json!(8);
    declined["state"] = serde_json::json!("DECLINED");
    mount_list(&server, serde_json::json!([draft, declined])).await;

    let out = bb(&server)
        .args(["pr", "list", "--state", "all"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("STATE"), "{text}");
    assert!(text.contains("Draft"), "{text}");
    assert!(text.contains("Declined"), "{text}");
}

#[tokio::test]
async fn list_json_emits_structured_reviewers_and_no_approvals_key() {
    let server = MockServer::start().await;
    mount_list(&server, serde_json::json!([pr_with_reviewers()])).await;

    let out = bb(&server).args(["pr", "list", "--json"]).output().unwrap();
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(value[0]["id"], 7);
    assert_eq!(value[0]["state"], "OPEN");
    assert_eq!(value[0]["draft"], false);
    assert_eq!(value[0]["reviewers"][0]["name"], "Ana");
    assert_eq!(value[0]["reviewers"][0]["uuid"], "{a}");
    assert_eq!(value[0]["reviewers"][0]["state"], "approved");
    assert_eq!(value[0]["reviewers"][1]["state"], "changes_requested");
    assert_eq!(value[0]["reviewers"][2]["state"], "pending");
    assert!(
        value[0].get("approvals").is_none(),
        "approvals key should be gone: {value}"
    );
}

#[tokio::test]
async fn list_json_on_zero_rows_is_a_pure_empty_array() {
    let server = MockServer::start().await;
    mount_list(&server, serde_json::json!([])).await;

    let out = bb(&server).args(["pr", "list", "--json"]).output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    let value: serde_json::Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("stdout did not parse as JSON: {e}\nstdout: {stdout}"));
    assert_eq!(value, serde_json::json!([]));
}

#[tokio::test]
async fn list_still_filters_by_destination_branch() {
    let server = MockServer::start().await;
    let mut other = pr_with_reviewers();
    other["id"] = serde_json::json!(8);
    other["source"] = serde_json::json!({ "branch": { "name": "feature/b" } });
    other["destination"] = serde_json::json!({ "branch": { "name": "develop" } });
    mount_list(&server, serde_json::json!([pr_with_reviewers(), other])).await;

    let out = bb(&server)
        .args(["pr", "list", "develop"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("feature/b"), "{text}");
    assert!(!text.contains("feature/a"), "destination filter not applied: {text}");
}

#[tokio::test]
async fn list_state_all_asks_the_api_for_every_state() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repositories/acme/widgets/pullrequests"))
        .and(query_param("state", "OPEN,MERGED,DECLINED,SUPERSEDED"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "values": [] })))
        .mount(&server)
        .await;

    bb(&server)
        .args(["pr", "list", "--state", "all"])
        .assert()
        .success();
}
```

`--state all` is needed by two of these tests and is a small, self-contained addition: Bitbucket accepts a repeated/comma `state` filter, so `all` expands to `OPEN,MERGED,DECLINED,SUPERSEDED`.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --test pr_list`
Expected: FAIL — no `fields` parameter is sent, so the strict mock does not match and the command errors; the state column and structured JSON do not exist.

- [ ] **Step 3: Create `src/commands/pr_list.rs`**

```rust
use crate::api::models::{PullRequest, ReviewerState};
use crate::commands::pr::Ctx;
use crate::error::Result;
use crate::output::{self, Format};
use serde::Serialize;

/// Bitbucket's paginated pull-request endpoint returns a reduced object that omits
/// reviewers, participants and the draft flag. They come back only when asked for
/// explicitly with a partial-response parameter.
///
/// The `+` must arrive url-encoded as `%2B`: a bare `+` in a query string decodes
/// as a space and bitbucket then ignores the whole parameter, which is exactly the
/// silent failure this feature exists to fix.
const REVIEWER_FIELDS: &str = "%2Bvalues.reviewers,%2Bvalues.participants,%2Bvalues.draft";

const ALL_STATES: &str = "OPEN,MERGED,DECLINED,SUPERSEDED";

#[derive(Debug, Serialize)]
struct PrRow {
    id: u64,
    title: String,
    /// The api's own value, so `--json` stays faithful to bitbucket.
    state: String,
    draft: bool,
    /// The one word the table shows, folding `draft` into `state`. Carried on the
    /// row rather than recomputed at render time, because filtering means the rows
    /// and the fetched pull requests are no longer index-aligned.
    #[serde(skip)]
    display_state: String,
    author: String,
    source: String,
    destination: String,
    reviewers: Vec<ReviewerState>,
    url: String,
}

fn to_row(pr: &PullRequest) -> PrRow {
    PrRow {
        id: pr.id,
        title: pr.title.clone().unwrap_or_default(),
        state: pr.state.clone().unwrap_or_else(|| "-".into()),
        draft: pr.draft,
        display_state: pr.display_state(),
        author: pr.author_name().to_string(),
        source: pr.source_branch().to_string(),
        destination: pr.destination_branch().to_string(),
        reviewers: pr.reviewer_states(),
        url: pr.html_url().to_string(),
    }
}

fn reviewer_cell(reviewers: &[ReviewerState]) -> String {
    reviewers
        .iter()
        .map(|r| format!("{} {}", r.name, r.state.mark()))
        .collect::<Vec<_>>()
        .join(", ")
}

/// `all` is a bb-level convenience, not a bitbucket state; everything else is
/// passed through upper-cased as before.
fn state_query(state: &str) -> String {
    if state.eq_ignore_ascii_case("all") {
        ALL_STATES.to_string()
    } else {
        state.to_uppercase()
    }
}

pub async fn list(ctx: &Ctx, destination: Option<String>, state: String) -> Result<()> {
    let spinner = output::spinner("fetching pull requests");
    let prs: Vec<PullRequest> = ctx
        .client
        .paginate(&ctx.path(&format!(
            "/pullrequests?state={}&pagelen=50&fields={REVIEWER_FIELDS}",
            urlencoding::encode(&state_query(&state))
        )))
        .await?;
    spinner.finish_and_clear();

    let rows: Vec<PrRow> = prs
        .iter()
        .filter(|pr| match destination.as_deref() {
            Some(branch) => pr.destination_branch() == branch,
            None => true,
        })
        .map(to_row)
        .collect();

    render(ctx, &rows)
}

fn render(ctx: &Ctx, rows: &[PrRow]) -> Result<()> {
    match ctx.format {
        Format::Json => output::print_json(&rows)?,
        Format::Human => output::print_table(
            &[
                "ID",
                "TITLE",
                "STATE",
                "SOURCE",
                "→",
                "TARGET",
                "AUTHOR",
                "REVIEWERS",
            ],
            rows.iter()
                .map(|r| {
                    vec![
                        r.id.to_string(),
                        r.title.clone(),
                        r.display_state.clone(),
                        r.source.clone(),
                        "→".into(),
                        r.destination.clone(),
                        r.author.clone(),
                        reviewer_cell(&r.reviewers),
                    ]
                })
                .collect(),
        ),
    }
    Ok(())
}
```

- [ ] **Step 4: Delete the old list, wire the new one**

- In `src/commands/pr.rs`, delete `PrRow` (lines 33-43), `to_row` (45-65) and `list` (67-118). Remove now-unused imports (`ReviewerRef` and `User` are still used by `default_reviewers`; check what the compiler says rather than guessing).
- In `src/commands/mod.rs` add `pub mod pr_list;` in alphabetical position (after `pr.rs`'s `pub mod pr;`, before `pub mod pr_comments;`).
- In `src/main.rs`, change the `PrCommand::List` arm to `commands::pr_list::list(&ctx, destination, state).await`.
- In `src/main.rs`, update the `--state` doc comment to `/// State filter: OPEN, MERGED, DECLINED, SUPERSEDED or ALL`.
- In `tests/pr.rs`, add `.env("BB_KEYRING_DISABLE", "1")` to the `bb()` helper. This file has been running against the real OS keyring; every other test file already sets it.
- Delete the now-duplicated list tests from `tests/pr.rs` — `pr_list_renders_a_table`, `pr_list_filters_by_destination_branch`, `pr_list_requests_open_state_by_default`, `pr_list_json_emits_an_array`, `pr_list_json_on_zero_rows_is_pure_empty_array`. `tests/pr_list.rs` covers all five, and the old ones would now fail on the strict `fields` mock. Leave the diff/files/commits/404 tests in place.

- [ ] **Step 5: Run the tests**

Run: `cargo test --test pr_list --test pr`
Expected: PASS.

- [ ] **Step 6: Full gate**

Run: `cargo fmt --all && cargo clippy --all-targets -- -D warnings && cargo test`
Expected: all green.

- [ ] **Step 7: Commit**

```bash
git add src/commands/pr_list.rs src/commands/pr.rs src/commands/mod.rs src/main.rs tests/pr_list.rs tests/pr.rs
git commit -m "fix(pr): request reviewer fields so pr list can show reviewers and state"
```

---

## Task 3: Name-to-user resolution

**Files:**
- Create: `src/users.rs`
- Modify: `src/lib.rs` — add `pub mod users;`
- Test: `tests/user_resolve.rs` (new)

Tested at the library level with `wiremock`, like `tests/api_client.rs`, using the shared `tests/support/mod.rs` `client_for` helper. No binary, so no keyring involvement at all.

**Interfaces:**
- Consumes: `bb_cli::api::Client` (`paginate`, `get_json`), `bb_cli::repo::RepoSlug` (`workspace: String`, `path()`), `bb_cli::api::models::User` with `User::name()` from Task 1.
- Produces:
  - `pub async fn resolve_user(client: &Client, slug: &RepoSlug, query: &str, extra: &[User]) -> Result<User>`
  - `pub async fn current_user(client: &Client) -> Result<User>` — `GET /user`, used by Task 4 and Task 5.

- [ ] **Step 1: Write the failing tests**

Create `tests/user_resolve.rs`:

```rust
#![allow(clippy::unwrap_used)] // test code is exempt from the unwrap/expect ban

mod support;

use bb_cli::error::BbError;
use bb_cli::repo::RepoSlug;
use bb_cli::users::resolve_user;
use support::client_for;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn slug() -> RepoSlug {
    RepoSlug::parse("acme/widgets").unwrap()
}

async fn mount_members(server: &MockServer, members: serde_json::Value) {
    Mock::given(method("GET"))
        .and(path("/workspaces/acme/members"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "values": members })))
        .mount(server)
        .await;
}

async fn mount_default_reviewers(server: &MockServer, reviewers: serde_json::Value) {
    Mock::given(method("GET"))
        .and(path("/repositories/acme/widgets/default-reviewers"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "values": reviewers })))
        .mount(server)
        .await;
}

/// A uuid is already exact, so resolution must not spend two api calls on it.
#[tokio::test]
async fn a_uuid_is_used_verbatim_without_any_lookup() {
    let server = MockServer::start().await;

    let user = resolve_user(&client_for(&server.uri()), &slug(), "{9a1b}", &[])
        .await
        .unwrap();
    assert_eq!(user.uuid.as_deref(), Some("{9a1b}"));
    assert!(
        server.received_requests().await.unwrap_or_default().is_empty(),
        "a uuid should not trigger a lookup"
    );
}

#[tokio::test]
async fn a_substring_of_the_display_name_resolves() {
    let server = MockServer::start().await;
    mount_members(
        &server,
        serde_json::json!([
            { "user": { "uuid": "{p}", "display_name": "Patrick Stein", "nickname": "patrick" } },
            { "user": { "uuid": "{r}", "display_name": "Raigon Doe", "nickname": "raigon" } }
        ]),
    )
    .await;
    mount_default_reviewers(&server, serde_json::json!([])).await;

    let user = resolve_user(&client_for(&server.uri()), &slug(), "patri", &[])
        .await
        .unwrap();
    assert_eq!(user.uuid.as_deref(), Some("{p}"));
}

#[tokio::test]
async fn an_ambiguous_query_errors_and_names_every_candidate() {
    let server = MockServer::start().await;
    mount_members(
        &server,
        serde_json::json!([
            { "user": { "uuid": "{1}", "display_name": "Ana Cruz" } },
            { "user": { "uuid": "{2}", "display_name": "Anastasia Ivanova" } }
        ]),
    )
    .await;
    mount_default_reviewers(&server, serde_json::json!([])).await;

    let err = resolve_user(&client_for(&server.uri()), &slug(), "ana", &[])
        .await
        .unwrap_err();
    match err {
        BbError::Config(message) => {
            assert!(message.contains("Ana Cruz"), "{message}");
            assert!(message.contains("Anastasia Ivanova"), "{message}");
            assert!(message.contains("uuid"), "no escape hatch offered: {message}");
        }
        other => panic!("unexpected: {other:?}"),
    }
}

/// Without this rule, a workspace containing both "ana" and "anastasia" makes the
/// shorter name permanently unaddressable.
#[tokio::test]
async fn an_exact_name_beats_a_longer_substring_match() {
    let server = MockServer::start().await;
    mount_members(
        &server,
        serde_json::json!([
            { "user": { "uuid": "{1}", "display_name": "Ana", "nickname": "ana" } },
            { "user": { "uuid": "{2}", "display_name": "Anastasia", "nickname": "anastasia" } }
        ]),
    )
    .await;
    mount_default_reviewers(&server, serde_json::json!([])).await;

    let user = resolve_user(&client_for(&server.uri()), &slug(), "ANA", &[])
        .await
        .unwrap();
    assert_eq!(user.uuid.as_deref(), Some("{1}"));
}

#[tokio::test]
async fn no_match_errors_naming_the_query() {
    let server = MockServer::start().await;
    mount_members(&server, serde_json::json!([])).await;
    mount_default_reviewers(&server, serde_json::json!([])).await;

    let err = resolve_user(&client_for(&server.uri()), &slug(), "nobody", &[])
        .await
        .unwrap_err();
    match err {
        BbError::Config(message) => {
            assert!(message.contains("nobody"), "{message}");
            assert!(message.contains("uuid"), "{message}");
        }
        other => panic!("unexpected: {other:?}"),
    }
}

/// An email cannot become a uuid — bitbucket's member listings do not expose email
/// addresses — so it must fail at resolution with the normal message rather than
/// later, inside a write.
#[tokio::test]
async fn an_email_is_not_special_cased() {
    let server = MockServer::start().await;
    mount_members(&server, serde_json::json!([])).await;
    mount_default_reviewers(&server, serde_json::json!([])).await;

    let err = resolve_user(&client_for(&server.uri()), &slug(), "ana@example.com", &[])
        .await
        .unwrap_err();
    assert!(matches!(err, BbError::Config(_)), "got {err:?}");
}

/// The token may lack workspace scope. That must not make reviewer removal
/// impossible, because the smaller pools are enough for the common case.
#[tokio::test]
async fn a_403_on_members_falls_back_to_the_remaining_pool() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/workspaces/acme/members"))
        .respond_with(ResponseTemplate::new(403))
        .mount(&server)
        .await;
    mount_default_reviewers(
        &server,
        serde_json::json!([{ "uuid": "{p}", "display_name": "Patrick Stein" }]),
    )
    .await;

    let user = resolve_user(&client_for(&server.uri()), &slug(), "patrick", &[])
        .await
        .unwrap();
    assert_eq!(user.uuid.as_deref(), Some("{p}"));
}

/// `extra` is how `pr reviewers remove` can name someone who is tagged on the pull
/// request but is in neither the member list nor the default reviewers.
#[tokio::test]
async fn the_extra_pool_is_searched_too() {
    let server = MockServer::start().await;
    mount_members(&server, serde_json::json!([])).await;
    mount_default_reviewers(&server, serde_json::json!([])).await;

    let extra: Vec<bb_cli::api::models::User> =
        serde_json::from_value(serde_json::json!([{ "uuid": "{x}", "display_name": "Ex Ternal" }]))
            .unwrap();
    let user = resolve_user(&client_for(&server.uri()), &slug(), "ternal", &extra)
        .await
        .unwrap();
    assert_eq!(user.uuid.as_deref(), Some("{x}"));
}
```

Note the shape difference the tests encode: `/workspaces/{ws}/members` returns objects that *wrap* a user (`{"user": {...}}`), while `/default-reviewers` returns user objects directly. The implementation must handle both.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --test user_resolve`
Expected: FAIL to compile — no module `bb_cli::users`.

- [ ] **Step 3: Implement `src/users.rs`**

```rust
use crate::api::models::User;
use crate::api::{repo_path, Client};
use crate::error::{BbError, Result};
use crate::repo::RepoSlug;
use serde::Deserialize;

/// `/workspaces/{ws}/members` wraps each user in a membership object, unlike
/// `/default-reviewers`, which returns users directly.
#[derive(Debug, Deserialize)]
struct Membership {
    user: Option<User>,
}

pub async fn current_user(client: &Client) -> Result<User> {
    client.get_json("/user").await
}

/// Everyone `query` could plausibly mean, deduplicated by uuid.
async fn candidates(client: &Client, slug: &RepoSlug, extra: &[User]) -> Result<Vec<User>> {
    let mut pool: Vec<User> = Vec::new();

    // The token may not carry workspace scope. That is not fatal: the smaller
    // pools below still cover the common cases.
    match client
        .paginate::<Membership>(&format!("/workspaces/{}/members?pagelen=100", slug.workspace))
        .await
    {
        Ok(memberships) => pool.extend(memberships.into_iter().filter_map(|m| m.user)),
        Err(BbError::Api { status: 403, .. }) | Err(BbError::Auth) => {}
        Err(other) => return Err(other),
    }

    let defaults: Vec<User> = client
        .paginate(&repo_path(slug, "/default-reviewers?pagelen=100"))
        .await?;
    pool.extend(defaults);

    for user in extra {
        pool.push(User {
            uuid: user.uuid.clone(),
            account_id: user.account_id.clone(),
            display_name: user.display_name.clone(),
            nickname: user.nickname.clone(),
        });
    }

    let mut seen: Vec<String> = Vec::new();
    pool.retain(|user| match user.uuid.as_deref() {
        Some(uuid) => {
            let fresh = !seen.iter().any(|s| s == uuid);
            if fresh {
                seen.push(uuid.to_string());
            }
            fresh
        }
        None => true,
    });

    Ok(pool)
}

fn matches(user: &User, needle: &str) -> bool {
    [user.display_name.as_deref(), user.nickname.as_deref()]
        .into_iter()
        .flatten()
        .any(|field| field.to_lowercase().contains(needle))
}

fn is_exact(user: &User, needle: &str) -> bool {
    [user.display_name.as_deref(), user.nickname.as_deref()]
        .into_iter()
        .flatten()
        .any(|field| field.to_lowercase() == needle)
}

/// One human-typed name to one user.
///
/// A `{uuid}` is already exact and is taken verbatim, with no api call. Anything
/// else is matched case-insensitively as a substring of the display name or
/// nickname. Emails are not accepted: a reviewer is written to the api as a uuid,
/// and bitbucket's member listings do not expose email addresses, so an email
/// could never be resolved — failing here beats failing inside a write.
pub async fn resolve_user(
    client: &Client,
    slug: &RepoSlug,
    query: &str,
    extra: &[User],
) -> Result<User> {
    let query = query.trim();
    if query.is_empty() {
        return Err(BbError::Config("empty user name".into()));
    }
    if query.starts_with('{') && query.ends_with('}') {
        return Ok(User {
            uuid: Some(query.to_string()),
            account_id: None,
            display_name: None,
            nickname: None,
        });
    }

    let needle = query.to_lowercase();
    let pool = candidates(client, slug, extra).await?;

    // An exact name wins outright, or a workspace holding both "ana" and
    // "anastasia" makes "ana" unaddressable forever.
    let mut found: Vec<User> = pool.into_iter().filter(|u| matches(u, &needle)).collect();
    if found.iter().any(|u| is_exact(u, &needle)) {
        found.retain(|u| is_exact(u, &needle));
    }

    match found.len() {
        1 => Ok(found.remove(0)),
        0 => Err(BbError::Config(format!(
            "no user matching `{query}` — pass a `{{uuid}}` to be exact"
        ))),
        _ => {
            let names: Vec<&str> = found.iter().map(|u| u.name()).collect();
            Err(BbError::Config(format!(
                "`{query}` matches {} people: {} — pass a `{{uuid}}` to be exact",
                names.len(),
                names.join(", ")
            )))
        }
    }
}
```

Add `pub mod users;` to `src/lib.rs`, in alphabetical position after `pub mod secret;`.

Note: an exact match on two different people is still ambiguous and correctly falls into the `_` arm.

- [ ] **Step 4: Run the tests**

Run: `cargo test --test user_resolve`
Expected: PASS, 8 tests.

- [ ] **Step 5: Full gate**

Run: `cargo fmt --all && cargo clippy --all-targets -- -D warnings && cargo test`
Expected: all green.

- [ ] **Step 6: Commit**

```bash
git add src/users.rs src/lib.rs tests/user_resolve.rs
git commit -m "feat(users): resolve a typed name to one bitbucket user"
```

---

## Task 4: `bb pr reviewers` list, add, remove

**Files:**
- Create: `src/commands/pr_reviewers.rs`
- Modify: `src/commands/mod.rs` — add `pub mod pr_reviewers;`
- Modify: `src/main.rs` — new `Reviewers` variant on `PrCommand` and its dispatch
- Test: `tests/pr_reviewers.rs` (new)

**Interfaces:**
- Consumes: `ReviewState`/`ReviewerState`/`PullRequest::reviewer_states()` (Task 1); `crate::users::resolve_user(client, slug, query, extra)` (Task 3); `pr::Ctx`.
- Produces: `pub async fn list(ctx: &Ctx, id: u64) -> Result<()>`, `pub async fn add(ctx: &Ctx, id: u64, names: &str) -> Result<()>`, `pub async fn remove(ctx: &Ctx, id: u64, names: &str) -> Result<()>` in `commands::pr_reviewers`. `names` is a comma-separated list, split and trimmed inside.

- [ ] **Step 1: Write the failing tests**

Create `tests/pr_reviewers.rs`:

```rust
#![allow(clippy::unwrap_used)] // test code is exempt from the unwrap/expect ban

use assert_cmd::Command;
use predicates::str::contains;
use wiremock::matchers::{body_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn bb(server: &MockServer) -> Command {
    let mut cmd = Command::cargo_bin("bb").unwrap();
    cmd.env("BB_EMAIL", "dev@example.com")
        .env("BB_TOKEN", "t0ken-value")
        .env("BB_API_BASE", server.uri())
        .env("BB_REPO", "acme/widgets")
        .env("NO_COLOR", "1")
        // Without this a test can reach the real OS keyring and destroy the
        // developer's stored token.
        .env("BB_KEYRING_DISABLE", "1");
    cmd
}

fn pr_body() -> serde_json::Value {
    serde_json::json!({
        "id": 7,
        "title": "fix the thing",
        "state": "OPEN",
        "reviewers": [
            { "uuid": "{a}", "display_name": "Ana" },
            { "uuid": "{r}", "display_name": "Raigon Doe" }
        ],
        "participants": [
            { "role": "REVIEWER", "state": "approved", "user": { "uuid": "{a}", "display_name": "Ana" } },
            { "role": "REVIEWER", "state": null, "user": { "uuid": "{r}", "display_name": "Raigon Doe" } }
        ]
    })
}

async fn mount_get_pr(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/repositories/acme/widgets/pullrequests/7"))
        .respond_with(ResponseTemplate::new(200).set_body_json(pr_body()))
        .mount(server)
        .await;
}

async fn mount_members(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/workspaces/acme/members"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "values": [
                { "user": { "uuid": "{p}", "display_name": "Patrick Stein", "nickname": "patrick" } },
                { "user": { "uuid": "{a}", "display_name": "Ana", "nickname": "ana" } },
                { "user": { "uuid": "{r}", "display_name": "Raigon Doe", "nickname": "raigon" } }
            ]
        })))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repositories/acme/widgets/default-reviewers"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "values": [] })))
        .mount(server)
        .await;
}

/// The PUT body must carry the title — bitbucket rejects the request without it —
/// and the complete new reviewer set, because there is no add-reviewer endpoint.
async fn mount_put_expecting(server: &MockServer, reviewers: serde_json::Value) {
    let mut response = pr_body();
    response["reviewers"] = reviewers.clone();
    Mock::given(method("PUT"))
        .and(path("/repositories/acme/widgets/pullrequests/7"))
        .and(body_json(serde_json::json!({
            "title": "fix the thing",
            "reviewers": reviewers
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(response))
        .expect(1)
        .mount(server)
        .await;
}

#[tokio::test]
async fn reviewers_list_shows_name_and_state() {
    let server = MockServer::start().await;
    mount_get_pr(&server).await;

    bb(&server)
        .args(["pr", "reviewers", "7"])
        .assert()
        .success()
        .stdout(contains("Ana"))
        .stdout(contains("approved"))
        .stdout(contains("Raigon Doe"))
        .stdout(contains("pending"));
}

#[tokio::test]
async fn reviewers_list_json_is_structured() {
    let server = MockServer::start().await;
    mount_get_pr(&server).await;

    let out = bb(&server)
        .args(["pr", "reviewers", "7", "--json"])
        .output()
        .unwrap();
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(value[0]["name"], "Ana");
    assert_eq!(value[0]["uuid"], "{a}");
    assert_eq!(value[0]["state"], "approved");
}

#[tokio::test]
async fn add_puts_the_union_of_old_and_new_reviewers() {
    let server = MockServer::start().await;
    mount_get_pr(&server).await;
    mount_members(&server).await;
    mount_put_expecting(
        &server,
        serde_json::json!([{ "uuid": "{a}" }, { "uuid": "{r}" }, { "uuid": "{p}" }]),
    )
    .await;

    bb(&server)
        .args(["pr", "reviewers", "add", "7", "patrick"])
        .assert()
        .success()
        .stdout(contains("Patrick Stein"));
}

/// Adding someone already tagged must not issue a write; `expect(0)` on the PUT
/// makes wiremock fail the test if one is sent.
#[tokio::test]
async fn add_of_an_existing_reviewer_sends_no_put() {
    let server = MockServer::start().await;
    mount_get_pr(&server).await;
    mount_members(&server).await;
    Mock::given(method("PUT"))
        .and(path("/repositories/acme/widgets/pullrequests/7"))
        .respond_with(ResponseTemplate::new(200).set_body_json(pr_body()))
        .expect(0)
        .mount(&server)
        .await;

    bb(&server)
        .args(["pr", "reviewers", "add", "7", "ana"])
        .assert()
        .success()
        .stdout(contains("already"));
}

#[tokio::test]
async fn remove_puts_the_reduced_set() {
    let server = MockServer::start().await;
    mount_get_pr(&server).await;
    mount_members(&server).await;
    mount_put_expecting(&server, serde_json::json!([{ "uuid": "{a}" }])).await;

    bb(&server)
        .args(["pr", "reviewers", "remove", "7", "raigon"])
        .assert()
        .success();
}

#[tokio::test]
async fn remove_of_someone_not_tagged_errors_and_sends_no_put() {
    let server = MockServer::start().await;
    mount_get_pr(&server).await;
    mount_members(&server).await;
    Mock::given(method("PUT"))
        .and(path("/repositories/acme/widgets/pullrequests/7"))
        .respond_with(ResponseTemplate::new(200).set_body_json(pr_body()))
        .expect(0)
        .mount(&server)
        .await;

    bb(&server)
        .args(["pr", "reviewers", "remove", "7", "patrick"])
        .assert()
        .code(1)
        .stderr(contains("not a reviewer"));
}

/// A typo in the second name must not leave a half-applied change, so every name
/// is resolved before anything is written.
#[tokio::test]
async fn one_bad_name_in_a_list_prevents_the_whole_put() {
    let server = MockServer::start().await;
    mount_get_pr(&server).await;
    mount_members(&server).await;
    Mock::given(method("PUT"))
        .and(path("/repositories/acme/widgets/pullrequests/7"))
        .respond_with(ResponseTemplate::new(200).set_body_json(pr_body()))
        .expect(0)
        .mount(&server)
        .await;

    bb(&server)
        .args(["pr", "reviewers", "add", "7", "patrick,nobodyhere"])
        .assert()
        .code(1)
        .stderr(contains("nobodyhere"));
}

#[tokio::test]
async fn a_missing_pr_exits_three() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repositories/acme/widgets/pullrequests/999"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    bb(&server)
        .args(["pr", "reviewers", "999"])
        .assert()
        .code(3);
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --test pr_reviewers`
Expected: FAIL — clap rejects the unknown `reviewers` subcommand.

- [ ] **Step 3: Implement `src/commands/pr_reviewers.rs`**

```rust
use crate::api::models::{PullRequest, ReviewerRef, ReviewerState, User};
use crate::commands::pr::Ctx;
use crate::error::{BbError, Result};
use crate::output::{self, Format};
use crate::users::resolve_user;

async fn fetch(ctx: &Ctx, id: u64) -> Result<PullRequest> {
    ctx.client
        .get_json(&ctx.path(&format!("/pullrequests/{id}")))
        .await
}

fn render(ctx: &Ctx, states: &[ReviewerState]) -> Result<()> {
    match ctx.format {
        Format::Json => output::print_json(&states)?,
        Format::Human => output::print_table(
            &["NAME", "STATE"],
            states
                .iter()
                .map(|s| {
                    vec![
                        s.name.clone(),
                        // The serialized name is the same vocabulary the --json
                        // output uses, so humans and scripts read one set of words.
                        serde_json::to_value(s.state)
                            .ok()
                            .and_then(|v| v.as_str().map(str::to_string))
                            .unwrap_or_else(|| "pending".into()),
                    ]
                })
                .collect(),
        ),
    }
    Ok(())
}

pub async fn list(ctx: &Ctx, id: u64) -> Result<()> {
    let pr = fetch(ctx, id).await?;
    render(ctx, &pr.reviewer_states())
}

fn split_names(names: &str) -> Vec<&str> {
    names
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect()
}

/// Resolves every name before any write, so a typo in the second name cannot
/// leave a half-applied change.
async fn resolve_all(ctx: &Ctx, names: &str, pool: &[User]) -> Result<Vec<User>> {
    let requested = split_names(names);
    if requested.is_empty() {
        return Err(BbError::Config("no reviewer name given".into()));
    }
    let mut resolved = Vec::new();
    for name in requested {
        resolved.push(resolve_user(&ctx.client, &ctx.slug, name, pool).await?);
    }
    Ok(resolved)
}

/// There is no add-reviewer or remove-reviewer endpoint, so the whole set is
/// written back. `title` is included because the api rejects a PUT without it;
/// every other field is omitted and left untouched.
async fn write_reviewers(ctx: &Ctx, id: u64, pr: &PullRequest, uuids: Vec<String>) -> Result<()> {
    let body = serde_json::json!({
        "title": pr.title.clone().unwrap_or_default(),
        "reviewers": uuids
            .into_iter()
            .map(|uuid| ReviewerRef { uuid })
            .collect::<Vec<_>>(),
    });
    let updated: PullRequest = ctx
        .client
        .put_json(&ctx.path(&format!("/pullrequests/{id}")), &body)
        .await?;
    render(ctx, &updated.reviewer_states())
}

fn current_uuids(pr: &PullRequest) -> Vec<String> {
    pr.reviewers.iter().filter_map(|r| r.uuid.clone()).collect()
}

pub async fn add(ctx: &Ctx, id: u64, names: &str) -> Result<()> {
    let pr = fetch(ctx, id).await?;
    let resolved = resolve_all(ctx, names, &pr.reviewers).await?;

    let mut uuids = current_uuids(&pr);
    let mut added: Vec<String> = Vec::new();
    let mut already: Vec<String> = Vec::new();
    for user in &resolved {
        let uuid = user.uuid.clone().ok_or_else(|| {
            BbError::Config(format!("`{}` has no uuid to tag", user.name()))
        })?;
        if uuids.contains(&uuid) {
            already.push(user.name().to_string());
        } else {
            uuids.push(uuid);
            added.push(user.name().to_string());
        }
    }

    if added.is_empty() {
        if !ctx.format.is_json() {
            output::info(&format!("already a reviewer: {}", already.join(", ")));
        }
        return render(ctx, &pr.reviewer_states());
    }

    if !ctx.format.is_json() {
        output::success(&format!("added {}", added.join(", ")));
        if !already.is_empty() {
            output::info(&format!("already a reviewer: {}", already.join(", ")));
        }
    }
    write_reviewers(ctx, id, &pr, uuids).await
}

pub async fn remove(ctx: &Ctx, id: u64, names: &str) -> Result<()> {
    let pr = fetch(ctx, id).await?;
    let resolved = resolve_all(ctx, names, &pr.reviewers).await?;

    let mut uuids = current_uuids(&pr);
    let mut removed: Vec<String> = Vec::new();
    for user in &resolved {
        // A silent no-op would let "remove Raigon" look like it worked when it
        // matched nobody on this pull request.
        let uuid = user
            .uuid
            .as_deref()
            .filter(|uuid| uuids.iter().any(|u| u == uuid))
            .ok_or_else(|| {
                BbError::Config(format!("`{}` is not a reviewer on #{id}", user.name()))
            })?
            .to_string();
        uuids.retain(|u| *u != uuid);
        removed.push(user.name().to_string());
    }

    if !ctx.format.is_json() {
        output::success(&format!("removed {}", removed.join(", ")));
    }
    write_reviewers(ctx, id, &pr, uuids).await
}
```

`ReviewerRef` already exists in `src/api/models.rs:207-210` as `{ uuid: String }` with `Serialize`; reuse it rather than adding a second type.

- [ ] **Step 4: Wire the CLI**

In `src/main.rs`, add to `enum PrCommand`:

```rust
    /// Show, add or remove the reviewers tagged on a pull request
    Reviewers {
        #[command(subcommand)]
        command: ReviewersCommand,
    },
```

and a new enum beside it:

```rust
#[derive(Subcommand)]
enum ReviewersCommand {
    /// List the reviewers on a pull request and what each has decided
    #[command(alias = "l", alias = "ls")]
    List { id: u64 },
    /// Tag one or more reviewers, comma-separated
    Add {
        id: u64,
        /// Reviewer names, comma-separated; a `{uuid}` is taken verbatim
        names: String,
    },
    /// Untag one or more reviewers, comma-separated
    #[command(alias = "rm")]
    Remove {
        id: u64,
        /// Reviewer names, comma-separated; a `{uuid}` is taken verbatim
        names: String,
    },
}
```

`bb pr reviewers 7` with no subcommand must work, because the tests use it and it is the obvious spelling. A bare `id` cannot coexist with a required subcommand, so make the subcommand optional and default to listing:

```rust
    /// Show, add or remove the reviewers tagged on a pull request
    Reviewers {
        /// Pull request id (omit when using add/remove)
        id: Option<u64>,
        #[command(subcommand)]
        command: Option<ReviewersCommand>,
    },
```

and dispatch:

```rust
                PrCommand::Reviewers { id, command } => match (id, command) {
                    (_, Some(ReviewersCommand::List { id })) => {
                        commands::pr_reviewers::list(&ctx, id).await
                    }
                    (_, Some(ReviewersCommand::Add { id, names })) => {
                        commands::pr_reviewers::add(&ctx, id, &names).await
                    }
                    (_, Some(ReviewersCommand::Remove { id, names })) => {
                        commands::pr_reviewers::remove(&ctx, id, &names).await
                    }
                    (Some(id), None) => commands::pr_reviewers::list(&ctx, id).await,
                    (None, None) => Err(bb_cli::error::BbError::Config(
                        "pass a pull request id, or `add`/`remove`".into(),
                    )),
                },
```

Add `pub mod pr_reviewers;` to `src/commands/mod.rs` after `pub mod pr_list;`.

If clap rejects an optional positional alongside an optional subcommand, drop the positional and use `#[command(args_conflicts_with_subcommands = true)]` on the `Reviewers` variant — that attribute exists for exactly this shape. Do not change the tests' command lines to work around it.

- [ ] **Step 5: Run the tests**

Run: `cargo test --test pr_reviewers`
Expected: PASS, 8 tests.

- [ ] **Step 6: Full gate**

Run: `cargo fmt --all && cargo clippy --all-targets -- -D warnings && cargo test`
Expected: all green.

- [ ] **Step 7: Commit**

```bash
git add src/commands/pr_reviewers.rs src/commands/mod.rs src/main.rs tests/pr_reviewers.rs
git commit -m "feat(pr): add bb pr reviewers list, add and remove"
```

---

## Task 5: `pr list` filters, plus README

**Files:**
- Modify: `src/commands/pr_list.rs` — `ListArgs`, filter application
- Modify: `src/main.rs` — the new flags and their dispatch
- Modify: `README.md`
- Test: `tests/pr_list.rs` — append

**Interfaces:**
- Consumes: `crate::users::{resolve_user, current_user}` (Task 3); `PullRequest::reviewer_states()`, `ReviewState` (Task 1); `list`/`to_row`/`render` from Task 2.
- Produces: `pub struct ListArgs { pub destination: Option<String>, pub state: String, pub reviewer: Option<String>, pub author: Option<String>, pub review_state: Option<ReviewStateArg>, pub needs_my_review: bool }` and `pub async fn list(ctx: &Ctx, args: ListArgs) -> Result<()>`. `pub enum ReviewStateArg { Approved, ChangesRequested, Pending }` lives in `pr_list.rs` and derives `clap::ValueEnum`.

- [ ] **Step 1: Write the failing tests**

Append to `tests/pr_list.rs`:

```rust
/// Two pull requests: 7 has Ana approving and Dee pending, 8 has only Dee.
fn pr_pair() -> serde_json::Value {
    let mut second = pr_with_reviewers();
    second["id"] = serde_json::json!(8);
    second["title"] = serde_json::json!("other thing");
    second["source"] = serde_json::json!({ "branch": { "name": "feature/b" } });
    second["author"] = serde_json::json!({ "nickname": "ana", "display_name": "Ana" });
    second["reviewers"] = serde_json::json!([{ "uuid": "{d}", "display_name": "Dee" }]);
    second["participants"] = serde_json::json!([
        { "role": "REVIEWER", "state": null, "user": { "uuid": "{d}", "display_name": "Dee" } }
    ]);
    serde_json::json!([pr_with_reviewers(), second])
}

async fn mount_members(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/workspaces/acme/members"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "values": [
                { "user": { "uuid": "{a}", "display_name": "Ana", "nickname": "ana" } },
                { "user": { "uuid": "{b}", "display_name": "Bo", "nickname": "bo" } },
                { "user": { "uuid": "{d}", "display_name": "Dee", "nickname": "dee" } }
            ]
        })))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repositories/acme/widgets/default-reviewers"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "values": [] })))
        .mount(server)
        .await;
}

/// `@me` and the review-state filters need to know who the token belongs to.
async fn mount_me(server: &MockServer, uuid: &str, name: &str) {
    Mock::given(method("GET"))
        .and(path("/user"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "uuid": uuid,
            "display_name": name,
            "nickname": name.to_lowercase()
        })))
        .mount(server)
        .await;
}

#[tokio::test]
async fn state_draft_keeps_only_drafts() {
    let server = MockServer::start().await;
    let mut draft = pr_with_reviewers();
    draft["draft"] = serde_json::json!(true);
    let mut plain = pr_with_reviewers();
    plain["id"] = serde_json::json!(8);
    plain["title"] = serde_json::json!("not a draft");
    mount_list(&server, serde_json::json!([draft, plain])).await;

    let out = bb(&server)
        .args(["pr", "list", "--state", "draft"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("fix the thing"), "{text}");
    assert!(!text.contains("not a draft"), "draft filter not applied: {text}");
}

/// `--state draft` still has to ask bitbucket for OPEN: DRAFT is not an api state.
#[tokio::test]
async fn state_draft_requests_open_from_the_api() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repositories/acme/widgets/pullrequests"))
        .and(query_param("state", "OPEN"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "values": [] })))
        .mount(&server)
        .await;

    bb(&server)
        .args(["pr", "list", "--state", "draft"])
        .assert()
        .success();
}

#[tokio::test]
async fn reviewer_filter_keeps_only_prs_that_person_reviews() {
    let server = MockServer::start().await;
    mount_list(&server, pr_pair()).await;
    mount_members(&server).await;

    let out = bb(&server)
        .args(["pr", "list", "--reviewer", "bo"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("fix the thing"), "{text}");
    assert!(!text.contains("other thing"), "reviewer filter not applied: {text}");
}

#[tokio::test]
async fn author_filter_keeps_only_that_authors_prs() {
    let server = MockServer::start().await;
    mount_list(&server, pr_pair()).await;
    mount_members(&server).await;

    let out = bb(&server)
        .args(["pr", "list", "--author", "ana"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("other thing"), "{text}");
    assert!(!text.contains("fix the thing"), "author filter not applied: {text}");
}

#[tokio::test]
async fn author_me_uses_the_authenticated_account() {
    let server = MockServer::start().await;
    mount_list(&server, pr_pair()).await;
    mount_me(&server, "{me}", "Ana").await;

    let out = bb(&server)
        .args(["pr", "list", "--author", "@me"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("other thing"), "{text}");
    assert!(!text.contains("fix the thing"), "@me not resolved: {text}");
}

#[tokio::test]
async fn needs_my_review_keeps_prs_where_i_have_not_approved() {
    let server = MockServer::start().await;
    // I am Bo, who requested changes on 7 and is not a reviewer on 8.
    mount_list(&server, pr_pair()).await;
    mount_me(&server, "{b}", "Bo").await;

    let out = bb(&server)
        .args(["pr", "list", "--needs-my-review"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("fix the thing"), "{text}");
    assert!(!text.contains("other thing"), "not-a-reviewer pr kept: {text}");
}

#[tokio::test]
async fn needs_my_review_drops_prs_i_already_approved() {
    let server = MockServer::start().await;
    // I am Ana, who approved 7 and does not review 8.
    mount_list(&server, pr_pair()).await;
    mount_me(&server, "{a}", "Ana").await;

    let out = bb(&server)
        .args(["pr", "list", "--needs-my-review"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(!text.contains("fix the thing"), "approved pr still listed: {text}");
    assert!(!text.contains("other thing"), "{text}");
}

#[tokio::test]
async fn review_state_approved_keeps_only_what_i_approved() {
    let server = MockServer::start().await;
    mount_list(&server, pr_pair()).await;
    mount_me(&server, "{a}", "Ana").await;

    let out = bb(&server)
        .args(["pr", "list", "--review-state", "approved"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("fix the thing"), "{text}");
    assert!(!text.contains("other thing"), "{text}");
}

#[tokio::test]
async fn review_state_changes_requested_matches_my_rejection() {
    let server = MockServer::start().await;
    mount_list(&server, pr_pair()).await;
    mount_me(&server, "{b}", "Bo").await;

    let out = bb(&server)
        .args(["pr", "list", "--review-state", "changes-requested"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("fix the thing"), "{text}");
    assert!(!text.contains("other thing"), "{text}");
}

/// Filters must AND, not OR.
#[tokio::test]
async fn two_filters_intersect() {
    let server = MockServer::start().await;
    mount_list(&server, pr_pair()).await;
    mount_members(&server).await;

    // Dee reviews both; only 8 is authored by Ana.
    let out = bb(&server)
        .args(["pr", "list", "--reviewer", "dee", "--author", "ana"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("other thing"), "{text}");
    assert!(!text.contains("fix the thing"), "filters ORed instead of ANDed: {text}");
}

#[tokio::test]
async fn an_invalid_review_state_is_rejected_before_any_request() {
    let server = MockServer::start().await;

    bb(&server)
        .args(["pr", "list", "--review-state", "nonsense"])
        .assert()
        .failure();
    assert!(
        server.received_requests().await.unwrap_or_default().is_empty(),
        "clap should reject the value before any http call"
    );
}

#[tokio::test]
async fn a_filter_matching_nothing_prints_an_empty_json_array() {
    let server = MockServer::start().await;
    mount_list(&server, pr_pair()).await;
    mount_members(&server).await;

    let out = bb(&server)
        .args(["pr", "list", "--reviewer", "bo", "--author", "ana", "--json"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let value: serde_json::Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("stdout did not parse as JSON: {e}\nstdout: {stdout}"));
    assert_eq!(value, serde_json::json!([]));
}
```

Note: `pr_with_reviewers()` has author `Sean B` / nickname `sean`, and the second pull request in `pr_pair()` is authored by `Ana`, so the author filter distinguishes them.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --test pr_list`
Expected: FAIL — clap rejects `--reviewer`, `--author`, `--review-state`, `--needs-my-review`, and `--state draft` is passed to the API verbatim.

- [ ] **Step 3: Implement the filters in `src/commands/pr_list.rs`**

Add at the top:

```rust
use crate::api::models::ReviewState;
use crate::users::{current_user, resolve_user};

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum ReviewStateArg {
    Approved,
    ChangesRequested,
    Pending,
}

impl ReviewStateArg {
    fn as_state(self) -> ReviewState {
        match self {
            Self::Approved => ReviewState::Approved,
            Self::ChangesRequested => ReviewState::ChangesRequested,
            Self::Pending => ReviewState::Pending,
        }
    }
}

#[derive(Debug, Default)]
pub struct ListArgs {
    pub destination: Option<String>,
    pub state: String,
    pub reviewer: Option<String>,
    pub author: Option<String>,
    pub review_state: Option<ReviewStateArg>,
    pub needs_my_review: bool,
}
```

`clap::ValueEnum` renders `ChangesRequested` as `changes-requested` on the command line, which is what the tests use.

Replace `state_query` and `list`:

```rust
/// `all` and `draft` are bb-level conveniences, not bitbucket states. `draft` is a
/// boolean on an OPEN pull request, so it asks for OPEN and filters afterwards.
fn state_query(state: &str) -> String {
    if state.eq_ignore_ascii_case("all") {
        ALL_STATES.to_string()
    } else if state.eq_ignore_ascii_case("draft") {
        "OPEN".to_string()
    } else {
        state.to_uppercase()
    }
}

/// The uuid of whoever the token belongs to, fetched at most once per invocation
/// and only when a filter actually needs it.
async fn my_uuid(ctx: &Ctx) -> Result<Option<String>> {
    Ok(current_user(&ctx.client).await?.uuid)
}

fn my_review_state(pr: &PullRequest, my_uuid: Option<&str>) -> Option<ReviewState> {
    let me = my_uuid?;
    pr.reviewer_states()
        .into_iter()
        .find(|r| r.uuid.as_deref() == Some(me))
        .map(|r| r.state)
}

pub async fn list(ctx: &Ctx, args: ListArgs) -> Result<()> {
    // Resolve everything the filters need before fetching, so a bad name fails
    // fast instead of after a paginated download.
    let reviewer_uuid = match args.reviewer.as_deref() {
        Some(name) => resolve_user(&ctx.client, &ctx.slug, name, &[])
            .await?
            .uuid,
        None => None,
    };
    let author_uuid = match args.author.as_deref() {
        Some("@me") => my_uuid(ctx).await?,
        Some(name) => resolve_user(&ctx.client, &ctx.slug, name, &[]).await?.uuid,
        None => None,
    };
    let me = if args.needs_my_review || args.review_state.is_some() {
        my_uuid(ctx).await?
    } else {
        None
    };

    let want_draft = args.state.eq_ignore_ascii_case("draft");

    let spinner = output::spinner("fetching pull requests");
    let prs: Vec<PullRequest> = ctx
        .client
        .paginate(&ctx.path(&format!(
            "/pullrequests?state={}&pagelen=50&fields={REVIEWER_FIELDS}",
            urlencoding::encode(&state_query(&args.state))
        )))
        .await?;
    spinner.finish_and_clear();

    let rows: Vec<PrRow> = prs
        .iter()
        .filter(|pr| match args.destination.as_deref() {
            Some(branch) => pr.destination_branch() == branch,
            None => true,
        })
        .filter(|pr| !want_draft || pr.draft)
        .filter(|pr| match reviewer_uuid.as_deref() {
            Some(uuid) => pr
                .reviewer_states()
                .iter()
                .any(|r| r.uuid.as_deref() == Some(uuid)),
            None => true,
        })
        .filter(|pr| match author_uuid.as_deref() {
            Some(uuid) => {
                pr.author.as_ref().and_then(|a| a.uuid.as_deref()) == Some(uuid)
            }
            None => true,
        })
        .filter(|pr| match args.review_state {
            Some(wanted) => my_review_state(pr, me.as_deref()) == Some(wanted.as_state()),
            None => true,
        })
        .filter(|pr| {
            if !args.needs_my_review {
                return true;
            }
            // I am a reviewer and I have not approved.
            matches!(
                my_review_state(pr, me.as_deref()),
                Some(ReviewState::ChangesRequested) | Some(ReviewState::Pending)
            )
        })
        .map(to_row)
        .collect();

    render(ctx, &rows)
}
```

The two tests that filter by author name mount `/workspaces/acme/members` returning a user whose `uuid` matches the fixture's author uuid — so the fixture pull requests need `author.uuid`. Add `"uuid": "{sean}"` to `pr_with_reviewers()`'s author and `"uuid": "{a}"` to the second pull request's author in the test file, and `{ "user": { "uuid": "{sean}", "display_name": "Sean B", "nickname": "sean" } }` to `mount_members`.

- [ ] **Step 4: Wire the flags in `src/main.rs`**

Replace the `List` variant:

```rust
    /// List pull requests
    #[command(alias = "l", alias = "ls")]
    List {
        /// Only show pull requests targeting this branch
        destination: Option<String>,
        /// State filter: OPEN, MERGED, DECLINED, SUPERSEDED, DRAFT or ALL
        #[arg(long, default_value = "OPEN")]
        state: String,
        /// Only pull requests this person is tagged to review
        #[arg(long)]
        reviewer: Option<String>,
        /// Only pull requests opened by this person; `@me` for yourself
        #[arg(long)]
        author: Option<String>,
        /// Your own review state on the pull request
        #[arg(long, value_enum)]
        review_state: Option<commands::pr_list::ReviewStateArg>,
        /// Only pull requests waiting on your review
        #[arg(long)]
        needs_my_review: bool,
    },
```

and its dispatch arm:

```rust
                PrCommand::List {
                    destination,
                    state,
                    reviewer,
                    author,
                    review_state,
                    needs_my_review,
                } => {
                    commands::pr_list::list(
                        &ctx,
                        commands::pr_list::ListArgs {
                            destination,
                            state,
                            reviewer,
                            author,
                            review_state,
                            needs_my_review,
                        },
                    )
                    .await
                }
```

- [ ] **Step 5: Run the tests**

Run: `cargo test --test pr_list`
Expected: PASS.

- [ ] **Step 6: Update the README**

In `README.md`, in the pull-request section, document:

```markdown
### Reviewers

    bb pr reviewers 682                     # who is tagged, and what each decided
    bb pr reviewers add 682 patrick         # tag someone (comma-separate for several)
    bb pr reviewers remove 682 raigon       # untag someone

Names are matched case-insensitively against workspace members and the repository's
default reviewers. An ambiguous name errors and lists the candidates; pass a
`{uuid}` to be exact.

### Filtering the list

    bb pr list --needs-my-review            # waiting on you
    bb pr list --reviewer patrick           # tagged for someone else
    bb pr list --author @me                 # yours
    bb pr list --state draft                # drafts only
    bb pr list --review-state approved      # ones you already approved

`bb pr list` shows a STATE column (Draft / Open / Merged / Declined) and a
REVIEWERS column marking each reviewer: `✓` approved, `✗` changes requested,
`·` no state yet.
```

Match the surrounding README's heading level and code-block style rather than copying the above verbatim if it clashes.

- [ ] **Step 7: Full gate**

Run: `cargo fmt --all && cargo clippy --all-targets -- -D warnings && cargo test`
Expected: all green.

- [ ] **Step 8: Check coverage did not regress**

Run: `cargo llvm-cov --all-features --workspace --summary-only`
Expected: TOTAL line coverage at or above 90%. If below, add tests for whichever new branch is uncovered; do not add exclusions.

- [ ] **Step 9: Commit**

```bash
git add src/commands/pr_list.rs src/main.rs tests/pr_list.rs README.md
git commit -m "feat(pr): filter pr list by reviewer, author, review state and draft"
```

---

## Task 6: Verify against the real Bitbucket API

**Files:** none — this is a manual verification gate.

Every earlier task proved behaviour against `wiremock` fixtures written from the API docs. The defect being fixed was invisible to exactly that kind of test for a whole release, so the feature is not done until it is seen working against the real service.

Run from a real Bitbucket checkout — `/Users/seanbaufeld/Documents/repos/mailgpt/solutions_console`, whose remote is `check24/solutions_console`. Note that a shell function named `bb` shadows the binary in that directory, so invoke the built binary by path.

- [ ] **Step 1: Build the branch binary**

```bash
cargo build --release
```

- [ ] **Step 2: Reviewers actually render**

Run the release binary's `pr list` in the real checkout and confirm the REVIEWERS column is non-empty for a pull request that has reviewers, with the state marks. This is the exact output that is empty on v0.9.4.

- [ ] **Step 3: Cross-check one pull request**

Run `pr reviewers <id>` for that pull request and confirm the names and states agree with the Bitbucket web UI.

- [ ] **Step 4: Confirm the JSON contract**

Run `pr list --json | jq '.[0].reviewers'` and confirm structured objects with `name`, `uuid` and `state`, and that no human-facing text leaked into stdout.

- [ ] **Step 5: Exercise the filters**

Run `pr list --needs-my-review` and `pr list --state draft` and sanity-check both against the web UI.

- [ ] **Step 6: Report**

Report what was seen, verbatim, including anything that disagreed with expectations. Do not run a write (`add`/`remove`) against a real pull request without asking first — it notifies real people.

---

## Done

After Task 6, hand off to `superpowers:finishing-a-development-branch`: open the pull request against `main`, and let release-plz's two-PR flow cut the release once it merges. The `feat:` commits in this branch make it a minor bump — 0.10.0.
