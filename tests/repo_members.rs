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
async fn a_forbidden_user_pool_is_reported_as_partial() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/workspaces/acme/members"))
        .respond_with(ResponseTemplate::new(403))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repositories/acme/widgets/permissions-config/users"))
        .respond_with(ResponseTemplate::new(403))
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
    // An effective default reviewer is not proof of access to this repository.
    assert_eq!(value["users"][0]["eligibility"], "unknown");
}

/// A 404 from a repository-scoped endpoint means the repository is gone, not
/// that one list is unreadable. It must keep its documented exit code.
#[tokio::test]
async fn a_missing_repository_is_not_reported_as_a_partial_pool() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/workspaces/acme/members"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "values": []
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repositories/acme/widgets/permissions-config/users"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    bb(&server)
        .args(["repo", "members", "--json"])
        .assert()
        .code(3);
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

/// The human table is how a person chooses reviewers, so it has to say what the
/// JSON says: which pools a row came from, and whether repository access is
/// proven or merely possible.
#[tokio::test]
async fn human_output_labels_sources_and_repository_access() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/workspaces/acme/members"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "values": [{ "user": { "uuid": "{m}", "display_name": "Member Only", "nickname": "member" } }]
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repositories/acme/widgets/permissions-config/users"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "values": [{ "user": { "uuid": "{d}", "display_name": "Direct Grant" } }]
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(
            "/repositories/acme/widgets/effective-default-reviewers",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "values": [] })))
        .mount(&server)
        .await;

    let out = bb(&server).args(["repo", "members"]).output().unwrap();
    assert!(out.status.success(), "view failed: {out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("REPOSITORY ACCESS"), "{stdout}");
    assert!(stdout.contains("Member Only"), "{stdout}");
    // A workspace member is not proof of access to this repository.
    assert!(stdout.contains("unknown"), "{stdout}");
    // A direct permission is.
    assert!(stdout.contains("explicit"), "{stdout}");
    assert!(stdout.contains("Direct Grant"), "{stdout}");
}

/// A partial pool is told to the reader in words, not left to be inferred from
/// a row count that looks complete.
#[tokio::test]
async fn human_output_names_the_pools_it_could_not_read() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/workspaces/acme/members"))
        .respond_with(ResponseTemplate::new(403))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repositories/acme/widgets/permissions-config/users"))
        .respond_with(ResponseTemplate::new(403))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(
            "/repositories/acme/widgets/effective-default-reviewers",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "values": [] })))
        .mount(&server)
        .await;

    let out = bb(&server).args(["repo", "members"]).output().unwrap();
    assert!(out.status.success(), "view failed: {out:?}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("could not read some user pools"),
        "{stderr}"
    );
    assert!(stderr.contains("workspace"), "{stderr}");
    assert!(stderr.contains("repository"), "{stderr}");
}
