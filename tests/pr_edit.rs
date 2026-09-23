#![allow(clippy::unwrap_used)]
use assert_cmd::Command;
use serde_json::Value;
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const PR_PATH: &str = "/repositories/acme/widgets/pullrequests/7";

fn bb(server: &MockServer) -> Command {
    let mut cmd = Command::cargo_bin("bb").unwrap();
    cmd.env("BB_NO_UPDATE_CHECK", "1")
        .env("BB_EMAIL", "dev@example.com")
        .env("BB_TOKEN", "t0ken-value")
        .env("BB_API_BASE", server.uri())
        .env("BB_REPO", "acme/widgets")
        .env("BB_SKILL_NO_AUTO_REFRESH", "1")
        .env("NO_COLOR", "1");
    cmd
}

fn pr_body(state: &str, title: &str, description: &str) -> Value {
    serde_json::json!({
        "id": 7,
        "title": title,
        "state": state,
        "description": description,
        "summary": { "raw": description, "markup": "markdown" },
        "source": { "branch": { "name": "feat/cache" } },
        "destination": { "branch": { "name": "main" } },
        "links": { "html": { "href": "https://bitbucket.org/acme/widgets/pull-requests/7" } }
    })
}

async fn mount_get(server: &MockServer, state: &str) {
    Mock::given(method("GET"))
        .and(path(PR_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_json(pr_body(
            state,
            "Add widget cache",
            "Caches widgets.",
        )))
        .mount(server)
        .await;
}

async fn put_bodies(server: &MockServer) -> Vec<Value> {
    server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.method.as_str() == "PUT")
        .map(|r| serde_json::from_slice(&r.body).unwrap())
        .collect()
}

#[tokio::test]
async fn title_only_edit_sends_no_description() {
    let server = MockServer::start().await;
    mount_get(&server, "OPEN").await;
    Mock::given(method("PUT"))
        .and(path(PR_PATH))
        .and(body_partial_json(
            serde_json::json!({ "title": "Cache widgets" }),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(pr_body(
            "OPEN",
            "Cache widgets",
            "Caches widgets.",
        )))
        .expect(1)
        .mount(&server)
        .await;

    let out = bb(&server)
        .args(["pr", "edit", "7", "--title", "Cache widgets"])
        .assert()
        .success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("#7"), "stdout: {stdout}");
    assert!(stdout.contains("title"), "stdout: {stdout}");

    let bodies = put_bodies(&server).await;
    assert_eq!(bodies.len(), 1);
    assert!(
        bodies[0].get("description").is_none(),
        "a title-only edit must not rewrite the description: {}",
        bodies[0]
    );
}

#[tokio::test]
async fn description_only_edit_resends_the_current_title() {
    let server = MockServer::start().await;
    mount_get(&server, "OPEN").await;
    Mock::given(method("PUT"))
        .and(path(PR_PATH))
        .and(body_partial_json(serde_json::json!({
            "title": "Add widget cache",
            "description": "Caches widgets for an hour."
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(pr_body(
            "OPEN",
            "Add widget cache",
            "Caches widgets for an hour.",
        )))
        .expect(1)
        .mount(&server)
        .await;

    bb(&server)
        .args([
            "pr",
            "edit",
            "7",
            "--description",
            "Caches widgets for an hour.",
        ])
        .assert()
        .success();
}

#[tokio::test]
async fn description_stdin_is_read_and_trailing_newline_trimmed() {
    let server = MockServer::start().await;
    mount_get(&server, "OPEN").await;
    Mock::given(method("PUT"))
        .and(path(PR_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_json(pr_body(
            "OPEN",
            "Add widget cache",
            "Line one\n\nLine two",
        )))
        .expect(1)
        .mount(&server)
        .await;

    bb(&server)
        .args(["pr", "edit", "7", "--description-stdin"])
        .write_stdin("Line one\n\nLine two\n")
        .assert()
        .success();

    let bodies = put_bodies(&server).await;
    assert_eq!(bodies[0]["description"], "Line one\n\nLine two");
}

#[tokio::test]
async fn empty_description_clears_it() {
    let server = MockServer::start().await;
    mount_get(&server, "OPEN").await;
    Mock::given(method("PUT"))
        .and(path(PR_PATH))
        .and(body_partial_json(serde_json::json!({ "description": "" })))
        .respond_with(ResponseTemplate::new(200).set_body_json(pr_body(
            "OPEN",
            "Add widget cache",
            "",
        )))
        .expect(1)
        .mount(&server)
        .await;

    bb(&server)
        .args(["pr", "edit", "7", "--description", ""])
        .assert()
        .success();
}

#[tokio::test]
async fn unchanged_text_writes_nothing_and_json_stays_pure() {
    let server = MockServer::start().await;
    mount_get(&server, "OPEN").await;
    Mock::given(method("PUT"))
        .and(path(PR_PATH))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;

    // Same title, and the same description with only a trailing newline added.
    let out = bb(&server)
        .args(["pr", "edit", "7", "--title", "Add widget cache", "--json"])
        .args(["--description", "Caches widgets.\n"])
        .assert()
        .success();
    let value: Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(value["id"], 7);
    assert_eq!(value["changed"], serde_json::json!([]));
}

#[tokio::test]
async fn edit_json_prints_only_the_row() {
    let server = MockServer::start().await;
    mount_get(&server, "OPEN").await;
    Mock::given(method("PUT"))
        .and(path(PR_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_json(pr_body(
            "OPEN",
            "Cache widgets",
            "New text",
        )))
        .expect(1)
        .mount(&server)
        .await;

    let out = bb(&server)
        .args(["pr", "edit", "7", "--title", "Cache widgets"])
        .args(["--description", "New text", "--json"])
        .assert()
        .success();
    let value: Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(value["title"], "Cache widgets");
    assert_eq!(value["description"], "New text");
    assert_eq!(
        value["url"],
        "https://bitbucket.org/acme/widgets/pull-requests/7"
    );
    assert_eq!(
        value["changed"],
        serde_json::json!(["title", "description"])
    );
}

#[tokio::test]
async fn edit_refuses_a_merged_pull_request_before_writing() {
    let server = MockServer::start().await;
    mount_get(&server, "MERGED").await;
    Mock::given(method("PUT"))
        .and(path(PR_PATH))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;

    let out = bb(&server)
        .args(["pr", "edit", "7", "--title", "Cache widgets"])
        .assert()
        .failure()
        .code(1);
    let stderr = String::from_utf8(out.get_output().stderr.clone()).unwrap();
    assert!(stderr.contains("MERGED"), "stderr: {stderr}");
}

#[tokio::test]
async fn no_flags_without_a_terminal_errors_before_any_request() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;

    let out = bb(&server)
        .args(["pr", "edit", "7"])
        .write_stdin("")
        .assert()
        .failure()
        .code(1);
    let stderr = String::from_utf8(out.get_output().stderr.clone()).unwrap();
    assert!(stderr.contains("--title"), "stderr: {stderr}");
    assert!(stderr.contains("--description-stdin"), "stderr: {stderr}");
}

#[tokio::test]
async fn blank_title_errors_before_any_request() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;

    bb(&server)
        .args(["pr", "edit", "7", "--title", "   "])
        .assert()
        .failure()
        .code(1);
}

#[tokio::test]
async fn description_and_description_stdin_conflict() {
    let server = MockServer::start().await;
    bb(&server)
        .args([
            "pr",
            "edit",
            "7",
            "--description",
            "x",
            "--description-stdin",
        ])
        .assert()
        .failure()
        .code(2);
}

#[tokio::test]
async fn missing_pull_request_exits_3() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(PR_PATH))
        .respond_with(ResponseTemplate::new(404).set_body_json(
            serde_json::json!({ "type": "error", "error": { "message": "Not found" } }),
        ))
        .mount(&server)
        .await;

    bb(&server)
        .args(["pr", "edit", "7", "--title", "Cache widgets"])
        .assert()
        .failure()
        .code(3);
}
