#![allow(clippy::unwrap_used)] // test code is exempt from the unwrap/expect ban

use assert_cmd::Command;
use predicates::str::contains;
use wiremock::matchers::{body_json, body_partial_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn bb(server: &MockServer) -> Command {
    let mut cmd = Command::cargo_bin("bb").unwrap();
    cmd.env("BB_EMAIL", "dev@example.com")
        .env("BB_TOKEN", "t0ken-value")
        .env("BB_API_BASE", server.uri())
        .env("BB_REPO", "acme/widgets")
        .env("NO_COLOR", "1");
    cmd
}

fn created(id: u64) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "content": { "raw": "looks good" },
        "user": { "display_name": "Me" },
        "created_on": "2026-08-04T10:00:00+00:00",
        "links": { "html": { "href": "https://bitbucket.org/acme/widgets/pull-requests/7#comment-900" } }
    })
}

#[tokio::test]
async fn posts_a_general_comment() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/repositories/acme/widgets/pullrequests/7/comments"))
        .and(body_json(
            serde_json::json!({ "content": { "raw": "looks good" } }),
        ))
        .respond_with(ResponseTemplate::new(201).set_body_json(created(900)))
        .expect(1)
        .mount(&server)
        .await;

    bb(&server)
        .args(["pr", "comment", "7", "--body", "looks good"])
        .assert()
        .success()
        .stdout(contains("900"));
}

#[tokio::test]
async fn posts_an_inline_comment_with_file_and_line() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/repositories/acme/widgets/pullrequests/7/comments"))
        .and(body_partial_json(serde_json::json!({
            "content": { "raw": "off by one" },
            "inline": { "path": "src/main.rs", "to": 42 }
        })))
        .respond_with(ResponseTemplate::new(201).set_body_json(created(901)))
        .expect(1)
        .mount(&server)
        .await;

    bb(&server)
        .args([
            "pr",
            "comment",
            "7",
            "--body",
            "off by one",
            "--file",
            "src/main.rs",
            "--line",
            "42",
        ])
        .assert()
        .success();
}

#[tokio::test]
async fn posts_a_reply_to_an_existing_comment() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/repositories/acme/widgets/pullrequests/7/comments"))
        .and(body_partial_json(serde_json::json!({
            "content": { "raw": "agreed" },
            "parent": { "id": 900 }
        })))
        .respond_with(ResponseTemplate::new(201).set_body_json(created(902)))
        .expect(1)
        .mount(&server)
        .await;

    bb(&server)
        .args([
            "pr",
            "comment",
            "7",
            "--body",
            "agreed",
            "--reply-to",
            "900",
        ])
        .assert()
        .success();
}

#[tokio::test]
async fn reads_the_body_from_stdin() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/repositories/acme/widgets/pullrequests/7/comments"))
        .and(body_json(
            serde_json::json!({ "content": { "raw": "from a pipe" } }),
        ))
        .respond_with(ResponseTemplate::new(201).set_body_json(created(903)))
        .expect(1)
        .mount(&server)
        .await;

    bb(&server)
        .args(["pr", "comment", "7", "--body-stdin"])
        .write_stdin("from a pipe\n")
        .assert()
        .success();
}

#[tokio::test]
async fn line_without_file_is_rejected_before_any_request() {
    let server = MockServer::start().await;
    bb(&server)
        .args(["pr", "comment", "7", "--body", "x", "--line", "42"])
        .assert()
        .failure()
        .stderr(contains("--file"));
}

#[tokio::test]
async fn empty_body_is_rejected() {
    let server = MockServer::start().await;
    bb(&server)
        .args(["pr", "comment", "7", "--body", "   "])
        .assert()
        .failure()
        .stderr(contains("empty"));
}

#[tokio::test]
async fn reply_cannot_be_combined_with_inline_location() {
    let server = MockServer::start().await;
    bb(&server)
        .args([
            "pr",
            "comment",
            "7",
            "--body",
            "x",
            "--reply-to",
            "900",
            "--file",
            "src/main.rs",
            "--line",
            "1",
        ])
        .assert()
        .failure()
        .stderr(contains("--reply-to"));
}

/// `--yes` is the approval, so the request goes out and nothing else is fetched:
/// the comment lookup exists only to fill the prompt a human would have seen.
#[tokio::test]
async fn resolve_and_its_reversal_hit_the_right_verbs() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(
            "/repositories/acme/widgets/pullrequests/7/comments/900/resolve",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "user": { "display_name": "Me" }
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(
            "/repositories/acme/widgets/pullrequests/7/comments/900",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(created(900)))
        .expect(0)
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path(
            "/repositories/acme/widgets/pullrequests/7/comments/901/resolve",
        ))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;

    bb(&server)
        .args(["pr", "resolve", "7", "900", "--yes"])
        .assert()
        .success()
        .stdout(contains("900"));
    // Reopening restores a reviewer's point rather than hiding one, so it needs
    // no approval.
    bb(&server)
        .args(["pr", "unresolve", "7", "901"])
        .assert()
        .success()
        .stdout(contains("901"));
}

/// The gate: with no terminal there is nobody to approve, so the command must
/// name the flag and leave the thread alone. `expect(0)` is the real assertion —
/// a gate that errors *after* resolving would be no gate at all.
#[tokio::test]
async fn resolve_without_approval_sends_no_request() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(
            "/repositories/acme/widgets/pullrequests/7/comments/900/resolve",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .expect(0)
        .mount(&server)
        .await;

    bb(&server)
        .args(["pr", "resolve", "7", "900"])
        .write_stdin("y\n") // a piped `yes` must not count as approval either
        .assert()
        .failure()
        .stderr(contains("--yes"));
}

/// The prompt describes the thread, so declining leaves nothing resolved — the
/// same guarantee, one step later. Approving cannot be driven from a piped stdin
/// (that is the point of the gate), so the terminal branch is covered by the
/// unit tests over `describe` in `src/commands/pr_comments.rs`.
#[tokio::test]
async fn resolve_json_stays_pure_when_the_gate_rejects() {
    let server = MockServer::start().await;
    let out = bb(&server)
        .args(["pr", "resolve", "7", "900", "--json"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(
        out.stdout.is_empty(),
        "stdout must stay empty in json mode: {}",
        String::from_utf8_lossy(&out.stdout)
    );
}

#[tokio::test]
async fn resolve_json_names_the_comment_and_the_pull_request() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(
            "/repositories/acme/widgets/pullrequests/7/comments/900/resolve",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path(
            "/repositories/acme/widgets/pullrequests/7/comments/900/resolve",
        ))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let out = bb(&server)
        .args(["pr", "resolve", "7", "900", "--yes", "--json"])
        .output()
        .unwrap();
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(value["resolved"], 900);
    assert_eq!(value["pull_request"], 7);

    let out = bb(&server)
        .args(["pr", "unresolve", "7", "900", "--json"])
        .output()
        .unwrap();
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(value["unresolved"], 900);
    assert_eq!(value["pull_request"], 7);
}

/// A thread that was already resolved answers 404, which must surface as exit 3
/// rather than a success the caller would read as "done".
#[tokio::test]
async fn resolving_an_unknown_comment_exits_three() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(
            "/repositories/acme/widgets/pullrequests/7/comments/404/resolve",
        ))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    bb(&server)
        .args(["pr", "resolve", "7", "404", "--yes"])
        .assert()
        .code(3);
}

#[tokio::test]
async fn comment_json_reports_the_new_id() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/repositories/acme/widgets/pullrequests/7/comments"))
        .respond_with(ResponseTemplate::new(201).set_body_json(created(904)))
        .mount(&server)
        .await;

    let out = bb(&server)
        .args(["pr", "comment", "7", "--body", "looks good", "--json"])
        .output()
        .unwrap();
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(value["id"], 904);
}
