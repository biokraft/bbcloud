#![allow(clippy::unwrap_used)]
use assert_cmd::Command;
use serde_json::Value;
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn bb(server: &MockServer) -> Command {
    let mut cmd = Command::cargo_bin("bb").unwrap();
    cmd.env("BB_EMAIL", "dev@example.com")
        .env("BB_TOKEN", "t0ken-value")
        .env("BB_API_BASE", server.uri())
        .env("BB_REPO", "acme/widgets")
        .env("BB_SKILL_NO_AUTO_REFRESH", "1")
        .env("NO_COLOR", "1");
    cmd
}

fn pr_body(state: &str, destination: &str) -> Value {
    serde_json::json!({
        "id": 7,
        "title": "Add widget cache",
        "state": state,
        "source": { "branch": { "name": "feat/cache" } },
        "destination": { "branch": { "name": destination } },
        "links": { "html": { "href": "https://bitbucket.org/acme/widgets/pull-requests/7" } }
    })
}

async fn mount_get(server: &MockServer, state: &str, destination: &str) {
    Mock::given(method("GET"))
        .and(path("/repositories/acme/widgets/pullrequests/7"))
        .respond_with(ResponseTemplate::new(200).set_body_json(pr_body(state, destination)))
        .mount(server)
        .await;
}

#[tokio::test]
async fn retarget_puts_the_new_destination_and_resends_the_title() {
    let server = MockServer::start().await;
    mount_get(&server, "OPEN", "develop").await;
    Mock::given(method("PUT"))
        .and(path("/repositories/acme/widgets/pullrequests/7"))
        .and(body_partial_json(serde_json::json!({
            "title": "Add widget cache",
            "destination": { "branch": { "name": "main" } }
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(pr_body("OPEN", "main")))
        .expect(1)
        .mount(&server)
        .await;

    let out = bb(&server)
        .args(["pr", "retarget", "7", "--to", "main"])
        .assert()
        .success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("develop"), "stdout: {stdout}");
    assert!(stdout.contains("main"), "stdout: {stdout}");
}

#[tokio::test]
async fn retarget_json_prints_only_the_serde_value() {
    let server = MockServer::start().await;
    mount_get(&server, "OPEN", "develop").await;
    Mock::given(method("PUT"))
        .and(path("/repositories/acme/widgets/pullrequests/7"))
        .respond_with(ResponseTemplate::new(200).set_body_json(pr_body("OPEN", "main")))
        .mount(&server)
        .await;

    let out = bb(&server)
        .args(["pr", "retarget", "7", "--to", "main", "--json"])
        .assert()
        .success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    let v: Value = serde_json::from_str(&stdout).unwrap(); // stdout must be pure json
    assert_eq!(v["id"], 7);
    assert_eq!(v["destination"], "main");
    assert_eq!(v["source"], "feat/cache");
}

#[tokio::test]
async fn retarget_to_the_current_destination_writes_nothing() {
    let server = MockServer::start().await;
    mount_get(&server, "OPEN", "main").await;
    Mock::given(method("PUT"))
        .and(path("/repositories/acme/widgets/pullrequests/7"))
        .respond_with(ResponseTemplate::new(200).set_body_json(pr_body("OPEN", "main")))
        .expect(0)
        .mount(&server)
        .await;

    bb(&server)
        .args(["pr", "retarget", "7", "--to", "main"])
        .assert()
        .success();
}

#[tokio::test]
async fn retarget_refuses_a_merged_pull_request_before_writing() {
    let server = MockServer::start().await;
    mount_get(&server, "MERGED", "develop").await;
    Mock::given(method("PUT"))
        .and(path("/repositories/acme/widgets/pullrequests/7"))
        .respond_with(ResponseTemplate::new(200).set_body_json(pr_body("OPEN", "main")))
        .expect(0)
        .mount(&server)
        .await;

    let out = bb(&server)
        .args(["pr", "retarget", "7", "--to", "main"])
        .assert()
        .failure();
    let stderr = String::from_utf8(out.get_output().stderr.clone()).unwrap();
    assert!(stderr.contains("MERGED"), "stderr: {stderr}");
}

#[tokio::test]
async fn retarget_to_the_current_destination_still_prints_json() {
    let server = MockServer::start().await;
    mount_get(&server, "OPEN", "main").await;
    Mock::given(method("PUT"))
        .and(path("/repositories/acme/widgets/pullrequests/7"))
        .respond_with(ResponseTemplate::new(200).set_body_json(pr_body("OPEN", "main")))
        .expect(0)
        .mount(&server)
        .await;

    let out = bb(&server)
        .args(["pr", "retarget", "7", "--to", "main", "--json"])
        .assert()
        .success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    let v: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(v["id"], 7);
    assert_eq!(v["destination"], "main");
}
