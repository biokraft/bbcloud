#![allow(clippy::unwrap_used)]
use assert_cmd::Command;
use predicates::str::contains;
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn bb(server: &MockServer) -> Command {
    let mut cmd = Command::cargo_bin("bb").unwrap();
    cmd.env("BB_NO_UPDATE_CHECK", "1");
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

/// `--reviewer` names the whole set, so the repository's default reviewers must
/// not be fetched at all — the point of the flag is that nobody arrives
/// uninvited.
#[tokio::test]
async fn reviewer_flag_replaces_the_default_reviewers() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/user"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "uuid": "{me}", "display_name": "Me"
        })))
        .mount(&server)
        .await;
    // Reached by the name resolver's pool, never as a source of reviewers.
    Mock::given(method("GET"))
        .and(path("/repositories/acme/widgets/default-reviewers"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "values": [
                { "uuid": "{me}", "display_name": "Me" },
                { "uuid": "{dana}", "display_name": "Dana Scully" },
                { "uuid": "{unwanted}", "display_name": "Unwanted Person" }
            ]
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/workspaces/acme/members"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "values": [] })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repositories/acme/widgets/permissions-config/users"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "values": [] })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/repositories/acme/widgets/pullrequests"))
        .and(body_partial_json(serde_json::json!({
            "reviewers": [{ "uuid": "{dana}" }]
        })))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({ "id": 20 })))
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
            "--reviewer",
            "dana",
        ])
        .assert()
        .success();
}

/// A `{uuid}` is taken verbatim: the member and default-reviewer listings are
/// never read, so an ambiguous or unlistable name is still addressable.
#[tokio::test]
async fn reviewer_flag_takes_a_uuid_verbatim_without_a_name_lookup() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/user"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "uuid": "{me}", "display_name": "Me"
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/repositories/acme/widgets/pullrequests"))
        .and(body_partial_json(serde_json::json!({
            "reviewers": [{ "uuid": "{dana}" }, { "uuid": "{ash}" }]
        })))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({ "id": 21 })))
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
            "--reviewer",
            "{dana},{ash}",
        ])
        .assert()
        .success();
}

/// Every name resolves before the POST, so an unresolvable one opens nothing —
/// otherwise the pull request exists with a reviewer set the user never chose.
#[tokio::test]
async fn an_unresolvable_reviewer_opens_no_pull_request() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/workspaces/acme/members"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "values": [] })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repositories/acme/widgets/permissions-config/users"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "values": [] })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repositories/acme/widgets/default-reviewers"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "values": [] })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/repositories/acme/widgets/pullrequests"))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({ "id": 22 })))
        .expect(0)
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
            "--reviewer",
            "nobody-here",
        ])
        .assert()
        .failure()
        .stderr(contains("nobody-here"));
}

/// The author cannot review their own pull request — bitbucket answers 400 — so
/// naming yourself is dropped rather than turned into a failed write.
#[tokio::test]
async fn reviewer_flag_drops_the_author() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/user"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "uuid": "{me}", "display_name": "Me"
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/repositories/acme/widgets/pullrequests"))
        .and(body_partial_json(serde_json::json!({ "reviewers": [] })))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({ "id": 23 })))
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
            "--reviewer",
            "{me}",
        ])
        .assert()
        .success();
}

/// Names are resolved once, not once per target, and every pull request in a
/// multi-target create carries the same set.
#[tokio::test]
async fn reviewer_flag_applies_to_every_target() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/user"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "uuid": "{me}", "display_name": "Me"
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/repositories/acme/widgets/pullrequests"))
        .and(body_partial_json(serde_json::json!({
            "reviewers": [{ "uuid": "{dana}" }]
        })))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({ "id": 24 })))
        .expect(2)
        .mount(&server)
        .await;

    bb(&server)
        .args([
            "pr",
            "create",
            "main,release/1.x",
            "feature/a",
            "--title",
            "t",
            "--reviewer",
            "{dana}",
        ])
        .assert()
        .success();
}
