#![allow(clippy::unwrap_used)]
use assert_cmd::Command;
use predicates::str::contains;
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn bb(server: &MockServer) -> Command {
    let mut cmd = Command::cargo_bin("bb").unwrap();
    cmd.env("BB_EMAIL", "dev@example.com")
        .env("BB_TOKEN", "t0ken-value")
        .env("BB_API_BASE", server.uri())
        .env("BB_REPO", "acme/widgets")
        .env("NO_COLOR", "1")
        .env("BB_KEYRING_DISABLE", "1");
    cmd
}

async fn mock_user_and_reviewers(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/user"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "uuid": "{me}", "display_name": "Me"
        })))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repositories/acme/widgets/default-reviewers"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "values": [
                { "uuid": "{me}", "display_name": "Me" },
                { "uuid": "{other}", "display_name": "Other" }
            ]
        })))
        .mount(server)
        .await;
}

#[tokio::test]
async fn create_uses_explicit_source_and_target() {
    let server = MockServer::start().await;
    mock_user_and_reviewers(&server).await;
    Mock::given(method("POST"))
        .and(path("/repositories/acme/widgets/pullrequests"))
        .and(body_partial_json(serde_json::json!({
            "source": { "branch": { "name": "feature/a" } },
            "destination": { "branch": { "name": "main" } }
        })))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
            "id": 12,
            "links": { "html": { "href": "https://bitbucket.org/acme/widgets/pull-requests/12" } }
        })))
        .expect(1)
        .mount(&server)
        .await;

    bb(&server)
        .args(["pr", "create", "main", "feature/a", "--title", "add thing"])
        .assert()
        .success()
        .stdout(contains("12"));
}

#[tokio::test]
async fn create_excludes_the_current_user_from_default_reviewers() {
    let server = MockServer::start().await;
    mock_user_and_reviewers(&server).await;
    Mock::given(method("POST"))
        .and(path("/repositories/acme/widgets/pullrequests"))
        .and(body_partial_json(serde_json::json!({
            "reviewers": [{ "uuid": "{other}" }]
        })))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({ "id": 12 })))
        .expect(1)
        .mount(&server)
        .await;

    bb(&server)
        .args(["pr", "create", "main", "feature/a", "--title", "t"])
        .assert()
        .success();
}

#[tokio::test]
async fn create_skips_reviewer_lookup_when_disabled() {
    let server = MockServer::start().await;
    // No /user or /default-reviewers mocks: they must not be called.
    Mock::given(method("POST"))
        .and(path("/repositories/acme/widgets/pullrequests"))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({ "id": 13 })))
        .expect(1)
        .mount(&server)
        .await;

    bb(&server)
        .args([
            "pr",
            "create",
            "main",
            "feature/a",
            "--title",
            "t",
            "--no-default-reviewers",
        ])
        .assert()
        .success();
}

#[tokio::test]
async fn create_defaults_the_title_when_none_given() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/repositories/acme/widgets/pullrequests"))
        .and(body_partial_json(serde_json::json!({
            "title": "Merge feature/a into main"
        })))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({ "id": 14 })))
        .expect(1)
        .mount(&server)
        .await;

    bb(&server)
        .args([
            "pr",
            "create",
            "main",
            "feature/a",
            "--no-default-reviewers",
        ])
        .assert()
        .success();
}

#[tokio::test]
async fn create_opens_one_pr_per_comma_separated_target() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/repositories/acme/widgets/pullrequests"))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({ "id": 15 })))
        .expect(2)
        .mount(&server)
        .await;

    bb(&server)
        .args([
            "pr",
            "create",
            "main,develop",
            "feature/a",
            "--title",
            "t",
            "--no-default-reviewers",
        ])
        .assert()
        .success();
}

#[tokio::test]
async fn create_deduplicates_repeated_targets() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/repositories/acme/widgets/pullrequests"))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({ "id": 18 })))
        .expect(1)
        .mount(&server)
        .await;

    bb(&server)
        .args([
            "pr",
            "create",
            "main,main",
            "feature/a",
            "--title",
            "t",
            "--no-default-reviewers",
        ])
        .assert()
        .success();
}

#[tokio::test]
async fn create_sends_description_only_when_provided() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/repositories/acme/widgets/pullrequests"))
        .and(body_partial_json(
            serde_json::json!({ "description": "why this change" }),
        ))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({ "id": 16 })))
        .expect(1)
        .mount(&server)
        .await;

    bb(&server)
        .args([
            "pr",
            "create",
            "main",
            "feature/a",
            "--title",
            "t",
            "--description",
            "why this change",
            "--no-default-reviewers",
        ])
        .assert()
        .success();
}

#[tokio::test]
async fn create_json_reports_ids_and_urls() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/repositories/acme/widgets/pullrequests"))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
            "id": 17,
            "links": { "html": { "href": "https://bitbucket.org/acme/widgets/pull-requests/17" } }
        })))
        .mount(&server)
        .await;

    let out = bb(&server)
        .args([
            "pr",
            "create",
            "main",
            "feature/a",
            "--title",
            "t",
            "--no-default-reviewers",
            "--json",
        ])
        .output()
        .unwrap();
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(value[0]["id"], 17);
    assert!(value[0]["url"]
        .as_str()
        .unwrap()
        .contains("pull-requests/17"));
}

/// A target of only separators leaves no branch to target.
#[tokio::test]
async fn create_rejects_a_target_that_is_only_commas() {
    let server = MockServer::start().await;
    bb(&server)
        .args(["pr", "create", ",,", "feature/x"])
        .assert()
        .failure()
        .stderr(contains("no target branch given"));
}

/// Opening a pull request from a branch onto itself is always a mistake.
#[tokio::test]
async fn create_rejects_a_source_equal_to_the_target() {
    let server = MockServer::start().await;
    bb(&server)
        .args(["pr", "create", "main", "main"])
        .assert()
        .failure()
        .stderr(contains("source and target are both `main`"));
}
