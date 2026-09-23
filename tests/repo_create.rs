#![allow(clippy::unwrap_used)] // test code is exempt from the unwrap/expect ban

use assert_cmd::Command;
use predicates::str::contains;
use wiremock::matchers::{body_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn bb(server: &MockServer) -> Command {
    let mut cmd = Command::cargo_bin("bb").unwrap();
    cmd.env("BB_NO_UPDATE_CHECK", "1");
    cmd.env("BB_EMAIL", "dev@example.com")
        .env("BB_TOKEN", "t0ken-value")
        .env("BB_API_BASE", server.uri())
        .env("BB_WORKSPACE", "acme")
        .env("BB_SKILL_NO_AUTO_REFRESH", "1")
        .env("NO_COLOR", "1");
    cmd
}

fn created() -> serde_json::Value {
    serde_json::json!({
        "slug": "api-gateway",
        "full_name": "acme/api-gateway",
        "is_private": true,
        "project": { "key": "ENG", "name": "Engineering" },
        "links": {
            "html": { "href": "https://bitbucket.org/acme/api-gateway" },
            "clone": [
                { "name": "https", "href": "https://bitbucket.org/acme/api-gateway.git" },
                { "name": "ssh", "href": "git@bitbucket.org:acme/api-gateway.git" }
            ]
        }
    })
}

/// Mounts the create endpoint expecting `expected` as the exact body, plus a
/// projects endpoint that must NOT be called.
async fn mock_create(server: &MockServer, expected: serde_json::Value) {
    Mock::given(method("POST"))
        .and(path("/repositories/acme/api-gateway"))
        .and(body_json(expected))
        .respond_with(ResponseTemplate::new(201).set_body_json(created()))
        .expect(1)
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/workspaces/acme/projects"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "values": [] })))
        .expect(0)
        .mount(server)
        .await;
}

#[tokio::test]
async fn creates_a_private_repository_and_sends_no_policy_knobs() {
    let server = MockServer::start().await;
    // The whole body. No scm, no fork_policy, no mainbranch, no has_wiki, no
    // has_issues: those are workspace policy, and a CLI that overrides policy
    // silently is worse than one that inherits it. `is_private` is the sole
    // exception, because omitting it can publish source code.
    mock_create(
        &server,
        serde_json::json!({ "is_private": true, "project": { "key": "ENG" } }),
    )
    .await;

    bb(&server)
        .args(["repo", "create", "api-gateway", "--project", "ENG"])
        .assert()
        .success()
        .stdout(contains("acme/api-gateway"))
        .stdout(contains("https://bitbucket.org/acme/api-gateway"))
        .stdout(contains("git@bitbucket.org:acme/api-gateway.git"));
}

#[tokio::test]
async fn public_flag_flips_is_private_to_false() {
    let server = MockServer::start().await;
    mock_create(
        &server,
        serde_json::json!({ "is_private": false, "project": { "key": "ENG" } }),
    )
    .await;

    bb(&server)
        .args([
            "repo",
            "create",
            "api-gateway",
            "--project",
            "ENG",
            "--public",
        ])
        .assert()
        .success();
}

#[tokio::test]
async fn description_is_sent_when_given_and_the_key_is_absent_when_not() {
    let server = MockServer::start().await;
    mock_create(
        &server,
        serde_json::json!({
            "is_private": true,
            "project": { "key": "ENG" },
            "description": "the edge"
        }),
    )
    .await;

    bb(&server)
        .args([
            "repo",
            "create",
            "api-gateway",
            "--project",
            "ENG",
            "--description",
            "the edge",
        ])
        .assert()
        .success();
    // The absent case is covered by the first test, whose body_json matcher is
    // exact and would reject a `description: null` key.
}

#[tokio::test]
async fn json_mode_prints_only_the_repository() {
    let server = MockServer::start().await;
    mock_create(
        &server,
        serde_json::json!({ "is_private": true, "project": { "key": "ENG" } }),
    )
    .await;

    let out = bb(&server)
        .args([
            "repo",
            "create",
            "api-gateway",
            "--project",
            "ENG",
            "--json",
        ])
        .output()
        .unwrap();
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(value["full_name"], "acme/api-gateway");
}

#[tokio::test]
async fn without_a_project_and_without_a_terminal_it_names_the_flag_and_creates_nothing() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/repositories/acme/api-gateway"))
        .respond_with(ResponseTemplate::new(201).set_body_json(created()))
        .expect(0)
        .mount(&server)
        .await;
    // The picker's lookup must not run either: there is no terminal to show it in.
    Mock::given(method("GET"))
        .and(path("/workspaces/acme/projects"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "values": [] })))
        .expect(0)
        .mount(&server)
        .await;

    bb(&server)
        .args(["repo", "create", "api-gateway"])
        .assert()
        .failure()
        .stderr(contains("--project"));
}

#[tokio::test]
async fn a_rejected_name_surfaces_the_api_message() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/repositories/acme/api-gateway"))
        .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
            "error": { "message": "Repository with this Slug and Owner already exists." }
        })))
        .mount(&server)
        .await;

    bb(&server)
        .args(["repo", "create", "api-gateway", "--project", "ENG"])
        .assert()
        .failure()
        .stderr(contains("already exists"));
}
