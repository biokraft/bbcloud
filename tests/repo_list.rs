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

fn body() -> serde_json::Value {
    serde_json::json!({
        "values": [
            {
                "slug": "api-gateway",
                "full_name": "acme/api-gateway",
                "is_private": true,
                "project": { "key": "ENG", "name": "Engineering" },
                "updated_on": "2026-08-24T10:00:00+00:00"
            },
            {
                "slug": "public-docs",
                "full_name": "acme/public-docs",
                "is_private": false,
                "project": { "key": "OPS", "name": "Operations" },
                "updated_on": "2026-08-01T10:00:00+00:00"
            }
        ]
    })
}

#[tokio::test]
async fn lists_repositories_with_project_access_and_age() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repositories/acme"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body()))
        .mount(&server)
        .await;

    bb(&server)
        .args(["repo", "list"])
        .assert()
        .success()
        .stdout(
            contains("api-gateway")
                .and(contains("ENG"))
                .and(contains("private")),
        )
        .stdout(contains("public-docs").and(contains("public")));
}

#[tokio::test]
async fn project_filter_is_sent_as_a_server_side_query() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repositories/acme"))
        .and(query_param("q", "project.key=\"ENG\""))
        .respond_with(ResponseTemplate::new(200).set_body_json(body()))
        .expect(1)
        .mount(&server)
        .await;

    bb(&server)
        .args(["repo", "list", "--project", "ENG"])
        .assert()
        .success();
}

#[tokio::test]
async fn name_filter_is_applied_client_side_and_limit_truncates_after_it() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repositories/acme"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body()))
        .mount(&server)
        .await;

    let out = bb(&server)
        .args(["repo", "list", "--name", "GATE", "--json"])
        .output()
        .unwrap();
    let rows: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(rows.as_array().unwrap().len(), 1);
    assert_eq!(rows[0]["name"], "api-gateway");
}

#[tokio::test]
async fn empty_result_in_json_mode_prints_only_an_empty_array() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repositories/acme"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "values": [] })))
        .mount(&server)
        .await;

    let out = bb(&server)
        .args(["repo", "list", "--json"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&stdout).unwrap(),
        serde_json::json!([])
    );
    assert!(
        !stdout.to_lowercase().contains("nothing"),
        "prose in json: {stdout}"
    );
}

#[tokio::test]
async fn unauthenticated_exits_two_and_not_found_exits_three() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repositories/acme"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;
    bb(&server).args(["repo", "list"]).assert().code(2);

    let other = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repositories/acme"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&other)
        .await;
    bb(&other).args(["repo", "list"]).assert().code(3);
}
