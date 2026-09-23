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
        .env("NO_COLOR", "1");
    cmd
}

async fn mock_branches(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/repositories/acme/widgets/refs/branches"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "values": [
                {
                    "name": "feature/login",
                    "target": {
                        "author": { "user": { "display_name": "Alice Smith" } },
                        "date": "2026-08-03T10:00:00+00:00"
                    }
                },
                {
                    "name": "hotfix/crash",
                    "target": {
                        "author": { "raw": "Bob Jones <bob@example.com>" },
                        "date": "2026-08-01T10:00:00+00:00"
                    }
                }
            ]
        })))
        .mount(server)
        .await;
}

#[tokio::test]
async fn lists_branches_with_owner_and_age() {
    let server = MockServer::start().await;
    mock_branches(&server).await;

    bb(&server)
        .args(["branch", "list"])
        .assert()
        .success()
        .stdout(contains("feature/login"))
        .stdout(contains("Alice Smith"))
        .stdout(contains("hotfix/crash"))
        .stdout(contains("Bob Jones"));
}

#[tokio::test]
async fn filters_by_user_case_insensitively() {
    let server = MockServer::start().await;
    mock_branches(&server).await;

    let out = bb(&server)
        .args(["branch", "list", "--user", "alice"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("feature/login"), "{text}");
    assert!(
        !text.contains("hotfix/crash"),
        "user filter not applied: {text}"
    );
}

#[tokio::test]
async fn filters_by_branch_name_substring() {
    let server = MockServer::start().await;
    mock_branches(&server).await;

    let out = bb(&server)
        .args(["branch", "list", "--name", "HOTFIX"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("hotfix/crash"), "{text}");
    assert!(
        !text.contains("feature/login"),
        "name filter not applied: {text}"
    );
}

#[tokio::test]
async fn json_output_is_an_array_of_branches() {
    let server = MockServer::start().await;
    mock_branches(&server).await;

    let out = bb(&server)
        .args(["branch", "list", "--json"])
        .output()
        .unwrap();
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(value.as_array().unwrap().len(), 2);
    assert_eq!(value[0]["branch"], "feature/login");
}
