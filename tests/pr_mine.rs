#![allow(clippy::unwrap_used)]

use assert_cmd::Command;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn bb(base: &str) -> Command {
    let mut cmd = Command::cargo_bin("bb").unwrap();
    cmd.env("BB_EMAIL", "me@example.com")
        .env("BB_TOKEN", "t0ken-value")
        .env("BB_API_BASE", base)
        .env("BB_KEYRING_DISABLE", "1")
        .env("NO_COLOR", "1");
    cmd
}

fn user_body() -> serde_json::Value {
    serde_json::json!({ "uuid": "{me}", "display_name": "Me" })
}

fn pr(id: u64, repo: &str, author_uuid: &str, reviewer_uuid: Option<&str>) -> serde_json::Value {
    let reviewers = match reviewer_uuid {
        Some(uuid) => serde_json::json!([{ "uuid": uuid, "display_name": "R" }]),
        None => serde_json::json!([]),
    };
    serde_json::json!({
        "id": id,
        "title": format!("pr {id}"),
        "state": "OPEN",
        "draft": false,
        "updated_on": "2026-08-10T09:00:00+00:00",
        "author": { "uuid": author_uuid, "display_name": "A" },
        "reviewers": reviewers,
        "participants": [],
        "source": { "branch": { "name": "feat" } },
        "destination": { "branch": { "name": "main" } },
        "links": { "html": { "href": format!("https://bitbucket.org/{repo}/pull-requests/{id}") } }
    })
}

fn page(values: Vec<serde_json::Value>) -> serde_json::Value {
    serde_json::json!({ "values": values })
}

async fn mock_user(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/user"))
        .respond_with(ResponseTemplate::new(200).set_body_json(user_body()))
        .mount(server)
        .await;
}

#[tokio::test]
async fn role_author_asks_only_the_authored_endpoint() {
    let server = MockServer::start().await;
    mock_user(&server).await;
    Mock::given(method("GET"))
        .and(path("/pullrequests/%7Bme%7D"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(page(vec![pr(42, "acme/api", "{me}", None)])),
        )
        .expect(1)
        .mount(&server)
        .await;
    // No repository enumeration may happen on the author-only path.
    Mock::given(method("GET"))
        .and(path("/workspaces"))
        .respond_with(ResponseTemplate::new(200).set_body_json(page(vec![])))
        .expect(0)
        .mount(&server)
        .await;

    let out = bb(&server.uri())
        .args(["pr", "mine", "--role", "author", "--json"])
        .assert()
        .success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    let value: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let rows = value["pull_requests"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["id"], 42);
    assert_eq!(rows[0]["repo"], "acme/api");
    assert_eq!(rows[0]["my_role"], "author");
    assert_eq!(rows[0]["updated_on"], "2026-08-10T09:00:00+00:00");
    assert!(value["partial"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn empty_json_prints_only_the_value() {
    let server = MockServer::start().await;
    mock_user(&server).await;
    Mock::given(method("GET"))
        .and(path("/pullrequests/%7Bme%7D"))
        .respond_with(ResponseTemplate::new(200).set_body_json(page(vec![])))
        .mount(&server)
        .await;

    let out = bb(&server.uri())
        .args(["pr", "mine", "--role", "author", "--json"])
        .assert()
        .success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    let value: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert!(value["pull_requests"].as_array().unwrap().is_empty());
    assert!(value["partial"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn state_is_passed_through_to_the_api() {
    let server = MockServer::start().await;
    mock_user(&server).await;
    Mock::given(method("GET"))
        .and(path("/pullrequests/%7Bme%7D"))
        .and(query_param("state", "MERGED"))
        .respond_with(ResponseTemplate::new(200).set_body_json(page(vec![])))
        .expect(1)
        .mount(&server)
        .await;

    bb(&server.uri())
        .args([
            "pr", "mine", "--role", "author", "--state", "merged", "--json",
        ])
        .assert()
        .success();
}

#[tokio::test]
async fn a_404_from_the_authored_endpoint_exits_three() {
    let server = MockServer::start().await;
    mock_user(&server).await;
    Mock::given(method("GET"))
        .and(path("/pullrequests/%7Bme%7D"))
        .respond_with(ResponseTemplate::new(404).set_body_json(serde_json::json!({})))
        .mount(&server)
        .await;

    bb(&server.uri())
        .args(["pr", "mine", "--role", "author", "--json"])
        .assert()
        .code(3);
}

#[tokio::test]
async fn human_output_names_the_repository() {
    let server = MockServer::start().await;
    mock_user(&server).await;
    Mock::given(method("GET"))
        .and(path("/pullrequests/%7Bme%7D"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(page(vec![pr(42, "acme/api", "{me}", None)])),
        )
        .mount(&server)
        .await;

    let out = bb(&server.uri())
        .args(["pr", "mine", "--role", "author"])
        .assert()
        .success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("acme/api"), "got {stdout}");
    assert!(stdout.contains("REPO"), "got {stdout}");
}
