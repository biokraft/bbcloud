#![allow(clippy::unwrap_used)]

use assert_cmd::Command;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn bb(server: &MockServer) -> Command {
    let mut cmd = Command::cargo_bin("bb").unwrap();
    cmd.env("BB_NO_UPDATE_CHECK", "1")
        .env("BB_EMAIL", "dev@example.com")
        .env("BB_TOKEN", "t0ken-value")
        .env("BB_API_BASE", server.uri())
        .env("BB_REPO", "acme/widgets")
        .env("BB_KEYRING_DISABLE", "1")
        .env("NO_COLOR", "1");
    cmd
}

#[tokio::test]
async fn lists_deduplicated_candidates_with_their_sources() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/workspaces/acme/members"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "values": [{ "user": { "uuid": "{one}", "display_name": "One", "nickname": "one" } }]
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repositories/acme/widgets/permissions-config/users"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "values": [{ "user": { "uuid": "{two}", "display_name": "Two" } }]
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(
            "/repositories/acme/widgets/effective-default-reviewers",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "values": [
                { "user": { "uuid": "{one}", "display_name": "One" } },
                { "user": { "uuid": "{three}", "display_name": "Three" } }
            ]
        })))
        .mount(&server)
        .await;

    let out = bb(&server)
        .args(["repo", "members", "--json"])
        .output()
        .unwrap();
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let users = value["users"].as_array().unwrap();
    assert_eq!(users.len(), 3);
    assert_eq!(users[0]["uuid"], "{one}");
    assert_eq!(
        users[0]["sources"],
        serde_json::json!(["workspace", "default_reviewer"])
    );
    assert_eq!(users[1]["sources"], serde_json::json!(["repository"]));
    assert!(value["partial"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn partial_user_pools_are_reported_without_polluting_json() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/workspaces/acme/members"))
        .respond_with(ResponseTemplate::new(403))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repositories/acme/widgets/permissions-config/users"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(
            "/repositories/acme/widgets/effective-default-reviewers",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "values": [{ "user": { "uuid": "{one}", "display_name": "One" } }]
        })))
        .mount(&server)
        .await;

    let out = bb(&server)
        .args(["repo", "members", "--json"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(
        value["partial"],
        serde_json::json!(["workspace", "repository"])
    );
    assert_eq!(value["users"][0]["uuid"], "{one}");
}

#[tokio::test]
async fn empty_user_pools_produce_an_empty_json_report() {
    let server = MockServer::start().await;
    for endpoint in [
        "/workspaces/acme/members",
        "/repositories/acme/widgets/permissions-config/users",
        "/repositories/acme/widgets/effective-default-reviewers",
    ] {
        Mock::given(method("GET"))
            .and(path(endpoint))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({ "values": [] })),
            )
            .mount(&server)
            .await;
    }

    let out = bb(&server)
        .args(["repo", "members", "--json"])
        .output()
        .unwrap();
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(value["users"], serde_json::json!([]));
    assert_eq!(value["partial"], serde_json::json!([]));
}

#[tokio::test]
async fn authentication_failure_stops_before_later_pools() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/workspaces/acme/members"))
        .respond_with(ResponseTemplate::new(401))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repositories/acme/widgets/permissions-config/users"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "values": [] })))
        .expect(0)
        .mount(&server)
        .await;

    bb(&server).args(["repo", "members"]).assert().code(2);
}
