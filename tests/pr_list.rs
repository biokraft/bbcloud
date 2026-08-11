#![allow(clippy::unwrap_used)] // test code is exempt from the unwrap/expect ban

use assert_cmd::Command;
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
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({ "values": values })),
        )
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
    assert!(
        !text.contains("Zed"),
        "plain participant rendered as reviewer: {text}"
    );
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
    assert!(
        !text.contains("feature/a"),
        "destination filter not applied: {text}"
    );
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
