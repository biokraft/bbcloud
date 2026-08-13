#![allow(clippy::unwrap_used)] // test code is exempt from the unwrap/expect ban

mod support;

use assert_cmd::Command;
use bb_cli::api::models::{BuildState, BuildStatus};
use wiremock::matchers::{method, path};
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

async fn mount(server: &MockServer, id: u64, body: serde_json::Value) {
    Mock::given(method("GET"))
        .and(path(format!(
            "/repositories/acme/widgets/pullrequests/{id}/statuses"
        )))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

fn statuses_body(states: &[&str]) -> serde_json::Value {
    serde_json::json!({
        "values": states
            .iter()
            .enumerate()
            .map(|(i, s)| serde_json::json!({
                "key": format!("KEY{i}"),
                "name": format!("Check {i}"),
                "state": s,
                "url": format!("https://bitbucket.org/build/{i}")
            }))
            .collect::<Vec<_>>()
    })
}

/// The api returns a paginated envelope; `paginate` must unwrap `values` and the
/// model must survive a missing field.
#[tokio::test]
async fn statuses_deserialise_from_the_api_envelope() {
    let server = MockServer::start().await;
    // One status omits `name` and `url` entirely, because a reporter may.
    let mut body = statuses_body(&["SUCCESSFUL", "FAILED"]);
    body["values"][0] = serde_json::json!({ "key": "PIPELINE", "state": "SUCCESSFUL" });

    Mock::given(method("GET"))
        .and(path("/repositories/acme/widgets/pullrequests/7/statuses"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;

    let client = support::client_for(&server.uri());
    let got: Vec<BuildStatus> = client
        .paginate("/repositories/acme/widgets/pullrequests/7/statuses")
        .await
        .unwrap();

    assert_eq!(got.len(), 2);
    assert_eq!(got[0].key.as_deref(), Some("PIPELINE"));
    assert_eq!(got[0].name, None);
    assert_eq!(got[1].name.as_deref(), Some("Check 1"));
    assert_eq!(BuildState::rollup(&got), BuildState::Failed);
}

#[tokio::test]
async fn build_lists_every_check() {
    let server = MockServer::start().await;
    mount(&server, 7, statuses_body(&["SUCCESSFUL", "FAILED"])).await;

    let out = bb(&server)
        .args(["pr", "build", "7"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8(out).unwrap();

    assert!(text.contains("KEY0"), "missing first check:\n{text}");
    assert!(text.contains("KEY1"), "missing second check:\n{text}");
    assert!(text.contains("Check 1"), "missing check name:\n{text}");
    assert!(
        text.contains("build: FAILED"),
        "missing rollup heading:\n{text}"
    );
}

#[tokio::test]
async fn build_json_shape() {
    let server = MockServer::start().await;
    mount(&server, 7, statuses_body(&["SUCCESSFUL", "FAILED"])).await;

    let out = bb(&server)
        .args(["pr", "build", "7", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let got: serde_json::Value = serde_json::from_slice(&out).unwrap();

    assert_eq!(got["build_state"], "failed");
    assert_eq!(got["statuses"].as_array().unwrap().len(), 2);
    assert_eq!(got["statuses"][1]["state"], "FAILED");
    assert_eq!(got["statuses"][1]["url"], "https://bitbucket.org/build/1");
}

#[tokio::test]
async fn build_with_no_checks_says_so() {
    let server = MockServer::start().await;
    mount(&server, 7, serde_json::json!({ "values": [] })).await;

    let out = bb(&server)
        .args(["pr", "build", "7"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8(out).unwrap();

    assert!(text.contains("no build statuses"), "got:\n{text}");
}

/// The empty path is where a stray prose line usually escapes into `--json`.
#[tokio::test]
async fn build_json_with_no_checks_is_pure() {
    let server = MockServer::start().await;
    mount(&server, 7, serde_json::json!({ "values": [] })).await;

    let out = bb(&server)
        .args(["pr", "build", "7", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let got: serde_json::Value = serde_json::from_slice(&out).unwrap();

    assert_eq!(got["build_state"], "none");
    assert_eq!(got["statuses"].as_array().unwrap().len(), 0);
    assert_eq!(
        got.as_object().unwrap().len(),
        2,
        "unexpected extra keys: {got}"
    );
}

#[tokio::test]
async fn build_on_a_missing_pr_exits_3() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repositories/acme/widgets/pullrequests/404/statuses"))
        .respond_with(ResponseTemplate::new(404).set_body_json(serde_json::json!({
            "error": { "message": "Pull request not found" }
        })))
        .mount(&server)
        .await;

    bb(&server).args(["pr", "build", "404"]).assert().code(3);
}
