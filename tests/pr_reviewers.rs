#![allow(clippy::unwrap_used)] // test code is exempt from the unwrap/expect ban

use assert_cmd::Command;
use predicates::str::contains;
use wiremock::matchers::{body_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn bb(server: &MockServer) -> Command {
    let mut cmd = Command::cargo_bin("bb").unwrap();
    cmd.env("BB_EMAIL", "dev@example.com")
        .env("BB_TOKEN", "t0ken-value")
        .env("BB_API_BASE", server.uri())
        .env("BB_REPO", "acme/widgets")
        .env("NO_COLOR", "1")
        // Without this a test can reach the real OS keyring and destroy the
        // developer's stored token.
        .env("BB_KEYRING_DISABLE", "1");
    cmd
}

fn pr_body() -> serde_json::Value {
    serde_json::json!({
        "id": 7,
        "title": "fix the thing",
        "state": "OPEN",
        "reviewers": [
            { "uuid": "{a}", "display_name": "Ana" },
            { "uuid": "{r}", "display_name": "Raigon Doe" }
        ],
        "participants": [
            { "role": "REVIEWER", "state": "approved", "user": { "uuid": "{a}", "display_name": "Ana" } },
            { "role": "REVIEWER", "state": null, "user": { "uuid": "{r}", "display_name": "Raigon Doe" } }
        ]
    })
}

async fn mount_get_pr(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/repositories/acme/widgets/pullrequests/7"))
        .respond_with(ResponseTemplate::new(200).set_body_json(pr_body()))
        .mount(server)
        .await;
}

async fn mount_members(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/workspaces/acme/members"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "values": [
                { "user": { "uuid": "{p}", "display_name": "Patrick Stein", "nickname": "patrick" } },
                { "user": { "uuid": "{a}", "display_name": "Ana", "nickname": "ana" } },
                { "user": { "uuid": "{r}", "display_name": "Raigon Doe", "nickname": "raigon" } }
            ]
        })))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repositories/acme/widgets/default-reviewers"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "values": [] })))
        .mount(server)
        .await;
}

/// The PUT body must carry the title — bitbucket rejects the request without it —
/// and the complete new reviewer set, because there is no add-reviewer endpoint.
async fn mount_put_expecting(server: &MockServer, reviewers: serde_json::Value) {
    let mut response = pr_body();
    response["reviewers"] = reviewers.clone();
    Mock::given(method("PUT"))
        .and(path("/repositories/acme/widgets/pullrequests/7"))
        .and(body_json(serde_json::json!({
            "title": "fix the thing",
            "reviewers": reviewers
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(response))
        .expect(1)
        .mount(server)
        .await;
}

#[tokio::test]
async fn reviewers_list_shows_name_and_state() {
    let server = MockServer::start().await;
    mount_get_pr(&server).await;

    bb(&server)
        .args(["pr", "reviewers", "7"])
        .assert()
        .success()
        .stdout(contains("Ana"))
        .stdout(contains("approved"))
        .stdout(contains("Raigon Doe"))
        .stdout(contains("pending"));
}

#[tokio::test]
async fn reviewers_list_json_is_structured() {
    let server = MockServer::start().await;
    mount_get_pr(&server).await;

    let out = bb(&server)
        .args(["pr", "reviewers", "7", "--json"])
        .output()
        .unwrap();
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(value[0]["name"], "Ana");
    assert_eq!(value[0]["uuid"], "{a}");
    assert_eq!(value[0]["state"], "approved");
}

#[tokio::test]
async fn add_puts_the_union_of_old_and_new_reviewers() {
    let server = MockServer::start().await;
    mount_get_pr(&server).await;
    mount_members(&server).await;
    mount_put_expecting(
        &server,
        serde_json::json!([{ "uuid": "{a}" }, { "uuid": "{r}" }, { "uuid": "{p}" }]),
    )
    .await;

    bb(&server)
        .args(["pr", "reviewers", "add", "7", "patrick"])
        .assert()
        .success()
        .stdout(contains("Patrick Stein"));
}

/// Adding someone already tagged must not issue a write; `expect(0)` on the PUT
/// makes wiremock fail the test if one is sent.
#[tokio::test]
async fn add_of_an_existing_reviewer_sends_no_put() {
    let server = MockServer::start().await;
    mount_get_pr(&server).await;
    mount_members(&server).await;
    Mock::given(method("PUT"))
        .and(path("/repositories/acme/widgets/pullrequests/7"))
        .respond_with(ResponseTemplate::new(200).set_body_json(pr_body()))
        .expect(0)
        .mount(&server)
        .await;

    bb(&server)
        .args(["pr", "reviewers", "add", "7", "ana"])
        .assert()
        .success()
        .stdout(contains("already"));
}

#[tokio::test]
async fn remove_puts_the_reduced_set() {
    let server = MockServer::start().await;
    mount_get_pr(&server).await;
    mount_members(&server).await;
    mount_put_expecting(&server, serde_json::json!([{ "uuid": "{a}" }])).await;

    bb(&server)
        .args(["pr", "reviewers", "remove", "7", "raigon"])
        .assert()
        .success();
}

#[tokio::test]
async fn remove_of_someone_not_tagged_errors_and_sends_no_put() {
    let server = MockServer::start().await;
    mount_get_pr(&server).await;
    mount_members(&server).await;
    Mock::given(method("PUT"))
        .and(path("/repositories/acme/widgets/pullrequests/7"))
        .respond_with(ResponseTemplate::new(200).set_body_json(pr_body()))
        .expect(0)
        .mount(&server)
        .await;

    bb(&server)
        .args(["pr", "reviewers", "remove", "7", "patrick"])
        .assert()
        .code(1)
        .stderr(contains("not a reviewer"));
}

/// A typo in the second name must not leave a half-applied change, so every name
/// is resolved before anything is written.
#[tokio::test]
async fn one_bad_name_in_a_list_prevents_the_whole_put() {
    let server = MockServer::start().await;
    mount_get_pr(&server).await;
    mount_members(&server).await;
    Mock::given(method("PUT"))
        .and(path("/repositories/acme/widgets/pullrequests/7"))
        .respond_with(ResponseTemplate::new(200).set_body_json(pr_body()))
        .expect(0)
        .mount(&server)
        .await;

    bb(&server)
        .args(["pr", "reviewers", "add", "7", "patrick,nobodyhere"])
        .assert()
        .code(1)
        .stderr(contains("nobodyhere"));
}

#[tokio::test]
async fn a_missing_pr_exits_three() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repositories/acme/widgets/pullrequests/999"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    bb(&server)
        .args(["pr", "reviewers", "999"])
        .assert()
        .code(3);
}
