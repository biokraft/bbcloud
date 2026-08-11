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
    Mock::given(method("DELETE"))
        .and(path(
            "/repositories/acme/widgets/pullrequests/7/comments/901/resolve",
        ))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;

    bb(&server)
        .args(["pr", "resolve", "7", "900"])
        .assert()
        .success()
        .stdout(contains("900"));
    bb(&server)
        .args(["pr", "unresolve", "7", "901"])
        .assert()
        .success()
        .stdout(contains("901"));
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
        .args(["pr", "resolve", "7", "900", "--json"])
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
        .args(["pr", "resolve", "7", "404"])
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
