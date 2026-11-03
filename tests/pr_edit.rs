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
async fn ready_marks_a_draft_ready_without_changing_content() {
    let server = MockServer::start().await;
    let mut current = pr_body("OPEN", "Add widget cache", "Caches widgets.");
    current["draft"] = Value::Bool(true);
    Mock::given(method("GET"))
        .and(path(PR_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_json(current))
        .expect(1)
        .mount(&server)
        .await;
    let mut updated = pr_body("OPEN", "Add widget cache", "Caches widgets.");
    updated["draft"] = Value::Bool(false);
    Mock::given(method("PUT"))
        .and(path(PR_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_json(updated))
        .expect(1)
        .mount(&server)
        .await;

    let out = bb(&server)
        .args(["pr", "edit", "7", "--ready", "--json"])
        .write_stdin("")
        .assert()
        .success();
    let value: Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(value["changed"], serde_json::json!(["draft"]));
    assert!(value.get("draft").is_none());
    assert_eq!(
        put_bodies(&server).await,
        vec![serde_json::json!({ "title": "Add widget cache", "draft": false })]
    );
}

#[tokio::test]
async fn draft_state_flags_handle_transitions_and_noops_in_both_formats() {
    for (flag, requested) in [("--ready", false), ("--draft", true)] {
        for current_draft in [false, true] {
            for json in [false, true] {
                let server = MockServer::start().await;
                let mut current = pr_body("OPEN", "Add widget cache", "Caches widgets.");
                current["draft"] = Value::Bool(current_draft);
                Mock::given(method("GET"))
                    .and(path(PR_PATH))
                    .respond_with(ResponseTemplate::new(200).set_body_json(current.clone()))
                    .expect(1)
                    .mount(&server)
                    .await;
                let changes = current_draft != requested;
                current["draft"] = Value::Bool(requested);
                Mock::given(method("PUT"))
                    .and(path(PR_PATH))
                    .respond_with(ResponseTemplate::new(200).set_body_json(current))
                    .expect(if changes { 1 } else { 0 })
                    .mount(&server)
                    .await;

                let mut cmd = bb(&server);
                cmd.args(["pr", "edit", "7", flag]).write_stdin("");
                if json {
                    cmd.arg("--json");
                }
                let out = cmd.assert().success();
                if json {
                    let value: Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
                    let changed = if changes {
                        serde_json::json!(["draft"])
                    } else {
                        serde_json::json!([])
                    };
                    assert_eq!(value["changed"], changed);
                    if requested {
                        assert_eq!(value["draft"], true);
                    } else {
                        assert!(value.get("draft").is_none());
                    }
                    assert_eq!(value["title"], "Add widget cache");
                    assert_eq!(value["description"], "Caches widgets.");
                } else {
                    let stdout = String::from_utf8_lossy(&out.get_output().stdout);
                    assert!(stdout.contains(if changes {
                        "updated: draft"
                    } else {
                        "nothing to do"
                    }));
                }
                let bodies = put_bodies(&server).await;
                if changes {
                    assert_eq!(
                        bodies,
                        vec![
                            serde_json::json!({ "title": "Add widget cache", "draft": requested })
                        ]
                    );
                } else {
                    assert!(bodies.is_empty());
                }
            }
        }
    }
}

#[tokio::test]
async fn draft_state_can_be_changed_with_title_and_description() {
    for (flag, draft) in [("--ready", false), ("--draft", true)] {
        let server = MockServer::start().await;
        let mut current = pr_body("OPEN", "Add widget cache", "Caches widgets.");
        current["draft"] = Value::Bool(!draft);
        Mock::given(method("GET"))
            .and(path(PR_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_json(current))
            .mount(&server)
            .await;
        let mut updated = pr_body("OPEN", "Cache widgets", "New text");
        updated["draft"] = Value::Bool(draft);
        Mock::given(method("PUT"))
            .and(path(PR_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_json(updated))
            .expect(1)
            .mount(&server)
            .await;

        let out = bb(&server)
            .args([
                "pr",
                "edit",
                "7",
                flag,
                "--title",
                "Cache widgets",
                "--description-stdin",
                "--json",
            ])
            .write_stdin("New text\n")
            .assert()
            .success();
        let value: Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
        assert_eq!(
            value["changed"],
            serde_json::json!(["title", "description", "draft"])
        );
        assert_eq!(
            put_bodies(&server).await,
            vec![
                serde_json::json!({ "title": "Cache widgets", "description": "New text", "draft": draft })
            ]
        );
    }
}

#[tokio::test]
async fn content_edits_leave_draft_status_alone() {
    for (flag, text, expected) in [
        (
            "--title",
            "Cache widgets",
            serde_json::json!({ "title": "Cache widgets" }),
        ),
        (
            "--description",
            "New text",
            serde_json::json!({ "title": "Add widget cache", "description": "New text" }),
        ),
    ] {
        let server = MockServer::start().await;
        let mut current = pr_body("OPEN", "Add widget cache", "Caches widgets.");
        current["draft"] = Value::Bool(true);
        Mock::given(method("GET"))
            .and(path(PR_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_json(current.clone()))
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path(PR_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_json(current))
            .expect(1)
            .mount(&server)
            .await;

        let out = bb(&server)
            .args(["pr", "edit", "7", flag, text, "--json"])
            .assert()
            .success();
        let value: Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
        assert_eq!(value["draft"], true);
        assert_eq!(put_bodies(&server).await, vec![expected]);
    }
}

#[tokio::test]
async fn draft_json_reports_the_returned_state_not_the_flag() {
    let server = MockServer::start().await;
    mount_get(&server, "OPEN").await;
    let mut updated = pr_body("OPEN", "Add widget cache", "Caches widgets.");
    updated["draft"] = Value::Bool(false);
    Mock::given(method("PUT"))
        .and(path(PR_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_json(updated))
        .expect(1)
        .mount(&server)
        .await;

    let out = bb(&server)
        .args(["pr", "edit", "7", "--draft", "--json"])
        .assert()
        .success();
    let value: Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert!(value.get("draft").is_none());
    assert_eq!(value["changed"], serde_json::json!(["draft"]));
}

#[tokio::test]
async fn ready_and_draft_conflict_before_any_request() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;
    bb(&server)
        .args(["pr", "edit", "7", "--ready", "--draft"])
        .assert()
        .failure()
        .code(1);
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn draft_state_flags_refuse_closed_pull_requests() {
    for state in ["MERGED", "DECLINED"] {
        for flag in ["--ready", "--draft"] {
            let server = MockServer::start().await;
            mount_get(&server, state).await;
            Mock::given(method("PUT"))
                .respond_with(ResponseTemplate::new(200))
                .expect(0)
                .mount(&server)
                .await;
            let out = bb(&server)
                .args(["pr", "edit", "7", flag])
                .assert()
                .failure()
                .code(1);
            assert!(String::from_utf8_lossy(&out.get_output().stderr).contains(state));
        }
    }
}

#[test]
fn edit_help_lists_draft_state_flags() {
    let out = Command::cargo_bin("bb")
        .unwrap()
        .env("BB_NO_UPDATE_CHECK", "1")
        .env("BB_SKILL_NO_AUTO_REFRESH", "1")
        .args(["pr", "edit", "--help"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&out.get_output().stdout);
    assert!(stdout.contains("--ready"));
    assert!(stdout.contains("--draft"));
    assert!(stdout.contains("ready for review"));
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
async fn description_flag_trims_trailing_newline_like_stdin() {
    let server = MockServer::start().await;
    mount_get(&server, "OPEN").await;
    Mock::given(method("PUT"))
        .and(path(PR_PATH))
        .and(body_partial_json(serde_json::json!({
            "description": "New text"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(pr_body(
            "OPEN",
            "Add widget cache",
            "New text",
        )))
        .expect(1)
        .mount(&server)
        .await;

    bb(&server)
        .args(["pr", "edit", "7", "--description", "New text\n"])
        .assert()
        .success();

    let bodies = put_bodies(&server).await;
    assert_eq!(bodies[0]["description"], "New text");
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
    assert!(stderr.contains("--ready"), "stderr: {stderr}");
    assert!(stderr.contains("--draft"), "stderr: {stderr}");
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
        .code(1);
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
