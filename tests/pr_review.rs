#![allow(clippy::unwrap_used)]
use assert_cmd::Command;
use wiremock::matchers::{method, path};
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

#[tokio::test]
async fn request_changes_and_its_reversal_hit_the_right_verbs() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(
            "/repositories/acme/widgets/pullrequests/7/request-changes",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path(
            "/repositories/acme/widgets/pullrequests/8/request-changes",
        ))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;

    bb(&server)
        .args(["pr", "request-changes", "7", "--yes"])
        .assert()
        .success();
    bb(&server)
        .args(["pr", "no-request-changes", "8", "--yes"])
        .assert()
        .success();
}

#[tokio::test]
async fn request_changes_without_yes_asks_instead_of_writing() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(
            "/repositories/acme/widgets/pullrequests/7/request-changes",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .expect(0)
        .mount(&server)
        .await;

    let out = bb(&server)
        .args(["pr", "request-changes", "7"])
        .assert()
        .failure();
    let stderr = String::from_utf8(out.get_output().stderr.clone()).unwrap();
    assert!(
        stderr.contains("--yes"),
        "the error must name the flag to use, got: {stderr}"
    );
}

#[tokio::test]
async fn withdrawing_without_yes_asks_instead_of_writing() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path(
            "/repositories/acme/widgets/pullrequests/8/request-changes",
        ))
        .respond_with(ResponseTemplate::new(204))
        .expect(0)
        .mount(&server)
        .await;

    let out = bb(&server)
        .args(["pr", "no-request-changes", "8"])
        .assert()
        .failure();
    let stderr = String::from_utf8(out.get_output().stderr.clone()).unwrap();
    assert!(
        stderr.contains("--yes"),
        "the error must name the flag to use, got: {stderr}"
    );
}

#[tokio::test]
async fn yes_keeps_json_stdout_a_bare_value() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(
            "/repositories/acme/widgets/pullrequests/7/request-changes",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .expect(1)
        .mount(&server)
        .await;

    let out = bb(&server)
        .args(["pr", "request-changes", "7", "--yes", "--json"])
        .assert()
        .success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    let value: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(value["requested_changes"], 7);
}
