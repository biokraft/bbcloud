#![allow(clippy::unwrap_used)] // test code is exempt from the unwrap/expect ban

use assert_cmd::Command;
use predicates::str::contains;
use wiremock::matchers::{method, path};
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

#[tokio::test]
async fn pr_diff_prints_raw_diff() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repositories/acme/widgets/pullrequests/7/diff"))
        .respond_with(ResponseTemplate::new(200).set_body_string("--- a/x\n+++ b/x\n+line\n"))
        .mount(&server)
        .await;

    bb(&server)
        .args(["pr", "diff", "7"])
        .assert()
        .success()
        .stdout(contains("+line"));
}

#[tokio::test]
async fn pr_files_lists_paths() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repositories/acme/widgets/pullrequests/7/diffstat"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "values": [
                { "status": "modified", "new": { "path": "src/main.rs" } },
                { "status": "removed", "old": { "path": "old.php" } }
            ]
        })))
        .mount(&server)
        .await;

    bb(&server)
        .args(["pr", "files", "7"])
        .assert()
        .success()
        .stdout(contains("src/main.rs"))
        .stdout(contains("old.php"));
}

#[tokio::test]
async fn pr_commits_lists_summaries() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repositories/acme/widgets/pullrequests/7/commits"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "values": [
                { "hash": "abc1234def", "summary": { "raw": "fix the thing\n" } }
            ]
        })))
        .mount(&server)
        .await;

    bb(&server)
        .args(["pr", "commits", "7"])
        .assert()
        .success()
        .stdout(contains("fix the thing"))
        .stdout(contains("abc1234"));
}

#[tokio::test]
async fn missing_pr_exits_three() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repositories/acme/widgets/pullrequests/999/diff"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    bb(&server).args(["pr", "diff", "999"]).assert().code(3);
}
