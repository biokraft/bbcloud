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
        .env("NO_COLOR", "1");
    cmd
}

fn pr_json(id: u64, source: &str, dest: &str) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "title": format!("pr {id}"),
        "state": "OPEN",
        "author": { "nickname": "sean", "display_name": "Sean B" },
        "source": { "branch": { "name": source } },
        "destination": { "branch": { "name": dest } },
        "links": { "html": { "href": format!("https://bitbucket.org/acme/widgets/pull-requests/{id}") } },
        "reviewers": [{ "uuid": "{r1}", "display_name": "Rev One" }],
        "participants": [{ "user": { "display_name": "Rev One" }, "state": "approved" }]
    })
}

#[tokio::test]
async fn pr_list_renders_a_table() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repositories/acme/widgets/pullrequests"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "values": [pr_json(7, "feature/a", "main"), pr_json(8, "feature/b", "develop")]
        })))
        .mount(&server)
        .await;

    bb(&server)
        .args(["pr", "list"])
        .assert()
        .success()
        .stdout(contains("7"))
        .stdout(contains("feature/a"))
        .stdout(contains("Rev One"));
}

#[tokio::test]
async fn pr_list_filters_by_destination_branch() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repositories/acme/widgets/pullrequests"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "values": [pr_json(7, "feature/a", "main"), pr_json(8, "feature/b", "develop")]
        })))
        .mount(&server)
        .await;

    let out = bb(&server)
        .args(["pr", "list", "develop"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("feature/b"), "{text}");
    assert!(
        !text.contains("feature/a"),
        "destination filter not applied: {text}"
    );
}

#[tokio::test]
async fn pr_list_requests_open_state_by_default() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repositories/acme/widgets/pullrequests"))
        .and(query_param("state", "OPEN"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "values": [] })))
        .mount(&server)
        .await;

    bb(&server).args(["pr", "list"]).assert().success();
}

#[tokio::test]
async fn pr_list_json_emits_an_array() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repositories/acme/widgets/pullrequests"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "values": [pr_json(7, "feature/a", "main")]
        })))
        .mount(&server)
        .await;

    let out = bb(&server).args(["pr", "list", "--json"]).output().unwrap();
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(value[0]["id"], 7);
    assert_eq!(value[0]["source"], "feature/a");
}

/// Zero-row `--json` must stay pure JSON: `print_table`'s "nothing to show" line
/// is not format-aware, so purity depends on the call site gating on Format.
#[tokio::test]
async fn pr_list_json_on_zero_rows_is_pure_empty_array() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repositories/acme/widgets/pullrequests"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "values": [] })))
        .mount(&server)
        .await;

    let out = bb(&server).args(["pr", "list", "--json"]).output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    let value: serde_json::Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("stdout did not parse as JSON: {e}\nstdout: {stdout}"));
    assert_eq!(value, serde_json::json!([]));
}

#[tokio::test]
async fn pr_diff_prints_raw_diff() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repositories/acme/widgets/pullrequests/7/diff"))
        .respond_with(ResponseTemplate::new(200).set_body_string("--- a/x\n+++ b/x\n+line\n"))
        .mount(&server)
        .await;

    bb(&server)
        .args(["pr", "diff", "7"])
        .assert()
        .success()
        .stdout(contains("+line"));
}

#[tokio::test]
async fn pr_files_lists_paths() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repositories/acme/widgets/pullrequests/7/diffstat"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "values": [
                { "status": "modified", "new": { "path": "src/main.rs" } },
                { "status": "removed", "old": { "path": "old.php" } }
            ]
        })))
        .mount(&server)
        .await;

    bb(&server)
        .args(["pr", "files", "7"])
        .assert()
        .success()
        .stdout(contains("src/main.rs"))
        .stdout(contains("old.php"));
}

#[tokio::test]
async fn pr_commits_lists_summaries() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repositories/acme/widgets/pullrequests/7/commits"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "values": [
                { "hash": "abc1234def", "summary": { "raw": "fix the thing\n" } }
            ]
        })))
        .mount(&server)
        .await;

    bb(&server)
        .args(["pr", "commits", "7"])
        .assert()
        .success()
        .stdout(contains("fix the thing"))
        .stdout(contains("abc1234"));
}

#[tokio::test]
async fn missing_pr_exits_three() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repositories/acme/widgets/pullrequests/999/diff"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    bb(&server).args(["pr", "diff", "999"]).assert().code(3);
}
