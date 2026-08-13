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

fn repos_page(names: &[&str]) -> serde_json::Value {
    page(
        names
            .iter()
            .map(|n| serde_json::json!({ "full_name": n }))
            .collect(),
    )
}

#[tokio::test]
async fn reviewer_side_keeps_only_pull_requests_i_review() {
    let server = MockServer::start().await;
    mock_user(&server).await;
    Mock::given(method("GET"))
        .and(path("/workspaces"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(page(vec![serde_json::json!({ "slug": "acme" })])),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repositories/acme"))
        .respond_with(ResponseTemplate::new(200).set_body_json(repos_page(&["acme/api"])))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repositories/acme/api/pullrequests"))
        .respond_with(ResponseTemplate::new(200).set_body_json(page(vec![
            pr(7, "acme/api", "{other}", Some("{me}")),
            pr(8, "acme/api", "{other}", Some("{someone-else}")),
        ])))
        .mount(&server)
        .await;

    let out = bb(&server.uri())
        .args(["pr", "mine", "--role", "reviewer", "--json"])
        .assert()
        .success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    let value: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let rows = value["pull_requests"].as_array().unwrap();
    assert_eq!(rows.len(), 1, "only the pr I review may survive: {stdout}");
    assert_eq!(rows[0]["id"], 7);
    assert_eq!(rows[0]["my_role"], "reviewer");
    assert_eq!(rows[0]["my_review_state"], "pending");
}

#[tokio::test]
async fn a_500_from_repositories_fails_the_whole_command() {
    let server = MockServer::start().await;
    mock_user(&server).await;
    Mock::given(method("GET"))
        .and(path("/workspaces"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(page(vec![serde_json::json!({ "slug": "acme" })])),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repositories/acme"))
        .respond_with(ResponseTemplate::new(500).set_body_json(serde_json::json!({})))
        .mount(&server)
        .await;

    bb(&server.uri())
        .args(["pr", "mine", "--role", "reviewer", "--json"])
        .assert()
        .code(1);
}

#[tokio::test]
async fn a_401_from_repositories_exits_two() {
    let server = MockServer::start().await;
    mock_user(&server).await;
    Mock::given(method("GET"))
        .and(path("/workspaces"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(page(vec![serde_json::json!({ "slug": "acme" })])),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repositories/acme"))
        .respond_with(ResponseTemplate::new(401).set_body_json(serde_json::json!({})))
        .mount(&server)
        .await;

    bb(&server.uri())
        .args(["pr", "mine", "--role", "reviewer", "--json"])
        .assert()
        .code(2);
}

#[tokio::test]
async fn authored_and_reviewed_dedupes_into_one_row_marked_both() {
    let server = MockServer::start().await;
    mock_user(&server).await;
    Mock::given(method("GET"))
        .and(path("/pullrequests/%7Bme%7D"))
        .respond_with(ResponseTemplate::new(200).set_body_json(page(vec![pr(
            7,
            "acme/api",
            "{me}",
            Some("{me}"),
        )])))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/workspaces"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(page(vec![serde_json::json!({ "slug": "acme" })])),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repositories/acme"))
        .respond_with(ResponseTemplate::new(200).set_body_json(repos_page(&["acme/api"])))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repositories/acme/api/pullrequests"))
        .respond_with(ResponseTemplate::new(200).set_body_json(page(vec![pr(
            7,
            "acme/api",
            "{me}",
            Some("{me}"),
        )])))
        .mount(&server)
        .await;

    let out = bb(&server.uri())
        .args(["pr", "mine", "--json"])
        .assert()
        .success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    let value: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let rows = value["pull_requests"].as_array().unwrap();
    assert_eq!(rows.len(), 1, "the same pr must appear once: {stdout}");
    assert_eq!(rows[0]["my_role"], "both");
}

#[tokio::test]
async fn workspace_flag_skips_workspace_enumeration() {
    let server = MockServer::start().await;
    mock_user(&server).await;
    Mock::given(method("GET"))
        .and(path("/workspaces"))
        .respond_with(ResponseTemplate::new(200).set_body_json(page(vec![])))
        .expect(0)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repositories/acme"))
        .respond_with(ResponseTemplate::new(200).set_body_json(repos_page(&["acme/api"])))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repositories/acme/api/pullrequests"))
        .respond_with(ResponseTemplate::new(200).set_body_json(page(vec![])))
        .mount(&server)
        .await;

    bb(&server.uri())
        .args([
            "pr",
            "mine",
            "--role",
            "reviewer",
            "--workspace",
            "acme",
            "--json",
        ])
        .assert()
        .success();
}

#[tokio::test]
async fn repo_limit_caps_the_fan_out() {
    let server = MockServer::start().await;
    mock_user(&server).await;
    Mock::given(method("GET"))
        .and(path("/repositories/acme"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(repos_page(&["acme/api", "acme/web"])),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repositories/acme/api/pullrequests"))
        .respond_with(ResponseTemplate::new(200).set_body_json(page(vec![])))
        .expect(1)
        .mount(&server)
        .await;
    // Second repository is beyond the limit and must never be asked.
    Mock::given(method("GET"))
        .and(path("/repositories/acme/web/pullrequests"))
        .respond_with(ResponseTemplate::new(200).set_body_json(page(vec![])))
        .expect(0)
        .mount(&server)
        .await;

    bb(&server.uri())
        .args([
            "pr",
            "mine",
            "--role",
            "reviewer",
            "--workspace",
            "acme",
            "--repo-limit",
            "1",
            "--json",
        ])
        .assert()
        .success();
}

#[tokio::test]
async fn repositories_are_requested_newest_first() {
    let server = MockServer::start().await;
    mock_user(&server).await;
    Mock::given(method("GET"))
        .and(path("/repositories/acme"))
        .and(query_param("role", "member"))
        .and(query_param("sort", "-updated_on"))
        .respond_with(ResponseTemplate::new(200).set_body_json(repos_page(&[])))
        .expect(1)
        .mount(&server)
        .await;

    bb(&server.uri())
        .args([
            "pr",
            "mine",
            "--role",
            "reviewer",
            "--workspace",
            "acme",
            "--json",
        ])
        .assert()
        .success();
}

#[tokio::test]
async fn an_unreadable_workspace_is_reported_not_fatal() {
    let server = MockServer::start().await;
    mock_user(&server).await;
    Mock::given(method("GET"))
        .and(path("/workspaces"))
        .respond_with(ResponseTemplate::new(200).set_body_json(page(vec![
            serde_json::json!({ "slug": "acme" }),
            serde_json::json!({ "slug": "locked" }),
        ])))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repositories/acme"))
        .respond_with(ResponseTemplate::new(200).set_body_json(repos_page(&["acme/api"])))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repositories/acme/api/pullrequests"))
        .respond_with(ResponseTemplate::new(200).set_body_json(page(vec![pr(
            7,
            "acme/api",
            "{other}",
            Some("{me}"),
        )])))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repositories/locked"))
        .respond_with(ResponseTemplate::new(403).set_body_json(serde_json::json!({})))
        .mount(&server)
        .await;

    let out = bb(&server.uri())
        .args(["pr", "mine", "--role", "reviewer", "--json"])
        .assert()
        .success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    let value: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(value["pull_requests"].as_array().unwrap().len(), 1);
    assert_eq!(value["partial"], serde_json::json!(["locked"]));
}

#[tokio::test]
async fn build_is_fetched_once_for_a_deduped_row() {
    let server = MockServer::start().await;
    mock_user(&server).await;
    Mock::given(method("GET"))
        .and(path("/pullrequests/%7Bme%7D"))
        .respond_with(ResponseTemplate::new(200).set_body_json(page(vec![pr(
            7,
            "acme/api",
            "{me}",
            Some("{me}"),
        )])))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/workspaces"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(page(vec![serde_json::json!({ "slug": "acme" })])),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repositories/acme"))
        .respond_with(ResponseTemplate::new(200).set_body_json(repos_page(&["acme/api"])))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repositories/acme/api/pullrequests"))
        .respond_with(ResponseTemplate::new(200).set_body_json(page(vec![pr(
            7,
            "acme/api",
            "{me}",
            Some("{me}"),
        )])))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repositories/acme/api/pullrequests/7/statuses"))
        .respond_with(ResponseTemplate::new(200).set_body_json(page(vec![
            serde_json::json!({ "key": "PIPE", "name": "p", "state": "FAILED" }),
        ])))
        .expect(1)
        .mount(&server)
        .await;

    let out = bb(&server.uri())
        .args(["pr", "mine", "--build", "--json"])
        .assert()
        .success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    let value: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let rows = value["pull_requests"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["build_state"], "failed");
    assert_eq!(rows[0]["build"].as_array().unwrap().len(), 1);
}
