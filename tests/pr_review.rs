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
        .args(["pr", "request-changes", "7"])
        .assert()
        .success();
    bb(&server)
        .args(["pr", "no-request-changes", "8"])
        .assert()
        .success();
}
