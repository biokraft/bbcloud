#![allow(clippy::unwrap_used)] // test code is exempt from the unwrap/expect ban

use assert_cmd::Command;
use predicates::prelude::PredicateBooleanExt;
use predicates::str::contains;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn bb(server: &MockServer) -> Command {
    let mut cmd = Command::cargo_bin("bb").unwrap();
    cmd.env("BB_EMAIL", "dev@example.com")
        .env("BB_TOKEN", "t0ken-value")
        .env("BB_API_BASE", server.uri())
        .env("BB_WORKSPACE", "acme")
        .env("BB_SKILL_NO_AUTO_REFRESH", "1")
        .env("NO_COLOR", "1");
    cmd
}

async fn mock_projects(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/workspaces/acme/projects"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "values": [
                { "key": "ENG", "name": "Engineering", "is_private": true },
                { "key": "OPS", "name": "Operations", "is_private": false }
            ]
        })))
        .mount(server)
        .await;
}

#[tokio::test]
async fn lists_projects_with_key_name_and_access() {
    let server = MockServer::start().await;
    mock_projects(&server).await;

    bb(&server)
        .args(["project", "list"])
        .assert()
        .success()
        .stdout(
            contains("ENG")
                .and(contains("Engineering"))
                .and(contains("private")),
        )
        .stdout(contains("OPS").and(contains("public")));
}

#[tokio::test]
async fn filters_projects_by_name_substring_case_insensitively() {
    let server = MockServer::start().await;
    mock_projects(&server).await;

    let out = bb(&server)
        .args(["project", "list", "--name", "engine"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("ENG"), "{stdout}");
    assert!(
        !stdout.contains("OPS"),
        "filter leaked a non-match: {stdout}"
    );
}

#[tokio::test]
async fn limit_truncates_after_filtering() {
    let server = MockServer::start().await;
    mock_projects(&server).await;

    let out = bb(&server)
        .args(["project", "list", "--limit", "1", "--json"])
        .output()
        .unwrap();
    let rows: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(rows.as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn follows_pagination() {
    let server = MockServer::start().await;
    let page_two = format!("{}/workspaces/acme/projects?page=2", server.uri());
    Mock::given(method("GET"))
        .and(path("/workspaces/acme/projects"))
        .and(query_param("page", "2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "values": [{ "key": "OPS", "name": "Operations", "is_private": false }]
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/workspaces/acme/projects"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "values": [{ "key": "ENG", "name": "Engineering", "is_private": true }],
            "next": page_two
        })))
        .mount(&server)
        .await;

    let out = bb(&server)
        .args(["project", "list", "--json"])
        .output()
        .unwrap();
    let rows: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(rows.as_array().unwrap().len(), 2, "second page not fetched");
}

#[tokio::test]
async fn empty_result_in_json_mode_prints_only_an_empty_array() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/workspaces/acme/projects"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "values": [] })))
        .mount(&server)
        .await;

    let out = bb(&server)
        .args(["project", "list", "--json"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let rows: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(rows, serde_json::json!([]));
    assert!(
        !stdout.to_lowercase().contains("nothing"),
        "prose escaped into json stdout: {stdout}"
    );
}

#[tokio::test]
async fn without_a_resolvable_workspace_it_names_the_flag_and_sends_nothing() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/workspaces/acme/projects"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "values": [] })))
        .expect(0)
        .mount(&server)
        .await;

    let mut cmd = Command::cargo_bin("bb").unwrap();
    // No BB_WORKSPACE, no BB_REPO, and a cwd that is not a git checkout, so
    // every source of a workspace is absent.
    let empty = tempfile::tempdir().unwrap();
    cmd.env_clear()
        .env("BB_EMAIL", "dev@example.com")
        .env("BB_TOKEN", "t0ken-value")
        .env("BB_API_BASE", server.uri())
        .env("BB_KEYRING_DISABLE", "1")
        .env("BB_SKILL_NO_AUTO_REFRESH", "1")
        .env("NO_COLOR", "1")
        .env("HOME", empty.path())
        .current_dir(empty.path())
        .args(["project", "list"])
        .assert()
        .failure()
        .stderr(contains("--workspace"));
}
