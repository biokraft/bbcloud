#![allow(clippy::unwrap_used)]
use assert_cmd::Command;
use predicates::str::contains;
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

fn comments_body() -> serde_json::Value {
    serde_json::json!({
        "values": [
            {
                "id": 1,
                "content": { "raw": "general remark" },
                "user": { "display_name": "Alice" },
                "created_on": "2026-08-01T10:00:00+00:00"
            },
            {
                "id": 2,
                "content": { "raw": "resolved inline note" },
                "user": { "display_name": "Bob" },
                "created_on": "2026-08-02T10:00:00+00:00",
                "inline": { "path": "src/lib.rs", "to": 42 },
                "resolution": {}
            },
            {
                "id": 3,
                "content": { "raw": "open inline note" },
                "user": { "display_name": "Carol" },
                "created_on": "2026-08-03T10:00:00+00:00",
                "inline": { "path": "src/main.rs", "from": 7 }
            },
            {
                "id": 4,
                "content": { "raw": "gone" },
                "user": { "display_name": "Dave" },
                "created_on": "2026-08-04T10:00:00+00:00",
                "deleted": true
            }
        ]
    })
}

async fn mock_pr_and_comments(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/repositories/acme/widgets/pullrequests/7"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": 7,
            "title": "fix the thing",
            "state": "OPEN",
            "author": { "display_name": "Sean B" },
            "source": { "branch": { "name": "feature/a" } },
            "destination": { "branch": { "name": "main" } },
            "links": { "html": { "href": "https://bitbucket.org/acme/widgets/pull-requests/7" } }
        })))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repositories/acme/widgets/pullrequests/7/comments"))
        .respond_with(ResponseTemplate::new(200).set_body_json(comments_body()))
        .mount(server)
        .await;
}

#[tokio::test]
async fn view_shows_header_general_and_inline_comments() {
    let server = MockServer::start().await;
    mock_pr_and_comments(&server).await;

    bb(&server)
        .args(["pr", "view", "7"])
        .assert()
        .success()
        .stdout(contains("fix the thing"))
        .stdout(contains("general remark"))
        .stdout(contains("src/lib.rs"))
        .stdout(contains("42"))
        .stdout(contains("open inline note"));
}

#[tokio::test]
async fn view_marks_deleted_comments_without_showing_the_body() {
    let server = MockServer::start().await;
    mock_pr_and_comments(&server).await;

    let out = bb(&server).args(["pr", "view", "7"]).output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("[deleted]"), "{text}");
    assert!(!text.contains("gone"), "deleted body leaked: {text}");
}

#[tokio::test]
async fn unresolved_filters_inline_but_keeps_general() {
    let server = MockServer::start().await;
    mock_pr_and_comments(&server).await;

    let out = bb(&server)
        .args(["pr", "view", "7", "--unresolved"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("open inline note"), "{text}");
    assert!(
        !text.contains("resolved inline note"),
        "resolved thread not filtered: {text}"
    );
    assert!(
        text.contains("general remark"),
        "general comments must survive the filter: {text}"
    );
}

#[tokio::test]
async fn view_json_splits_general_and_inline() {
    let server = MockServer::start().await;
    mock_pr_and_comments(&server).await;

    let out = bb(&server)
        .args(["pr", "view", "7", "--json"])
        .output()
        .unwrap();
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(value["pull_request"]["id"], 7);
    let general = value["general"].as_array().unwrap();
    let inline = value["inline"].as_array().unwrap();
    assert_eq!(general.len(), 2, "general: {general:?}");
    assert_eq!(inline.len(), 2, "inline: {inline:?}");
    assert_eq!(inline[0]["file"], "src/lib.rs");
    assert_eq!(inline[0]["resolved"], true);
}

#[tokio::test]
async fn comments_are_ordered_oldest_first() {
    let server = MockServer::start().await;
    mock_pr_and_comments(&server).await;

    let out = bb(&server)
        .args(["pr", "view", "7", "--json"])
        .output()
        .unwrap();
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let ids: Vec<u64> = value["inline"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_u64().unwrap())
        .collect();
    assert_eq!(ids, vec![2, 3]);
}

#[tokio::test]
async fn comments_only_skips_the_pull_request_lookup() {
    let server = MockServer::start().await;
    // Only the comments endpoint is mocked; requesting the PR would 404 the run.
    Mock::given(method("GET"))
        .and(path("/repositories/acme/widgets/pullrequests/7/comments"))
        .respond_with(ResponseTemplate::new(200).set_body_json(comments_body()))
        .mount(&server)
        .await;

    bb(&server)
        .args(["pr", "view", "7", "--comments-only"])
        .assert()
        .success()
        .stdout(contains("general remark"));
}
