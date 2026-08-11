#![allow(clippy::unwrap_used)] // test code is exempt from the unwrap/expect ban

//! CLI-level pin for the incomplete-pool warning `resolve_user` emits on
//! `output::warn` (stderr). The library-level tests in `tests/user_resolve.rs`
//! cannot observe stderr in-process, so this file spawns the real binary and
//! reads it directly — this is the test that fails if the warning starts
//! firing on the success path, or stops firing on the failure path.

use assert_cmd::Command;
use wiremock::matchers::{method, path};
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

async fn mount_members_403(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/workspaces/acme/members"))
        .respond_with(ResponseTemplate::new(403))
        .mount(server)
        .await;
}

async fn mount_default_reviewers(server: &MockServer, reviewers: serde_json::Value) {
    Mock::given(method("GET"))
        .and(path("/repositories/acme/widgets/default-reviewers"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({ "values": reviewers })),
        )
        .mount(server)
        .await;
}

async fn mount_permissions_config(server: &MockServer, entries: serde_json::Value) {
    Mock::given(method("GET"))
        .and(path("/repositories/acme/widgets/permissions-config/users"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({ "values": entries })),
        )
        .mount(server)
        .await;
}

async fn mount_list(server: &MockServer, values: serde_json::Value) {
    Mock::given(method("GET"))
        .and(path("/repositories/acme/widgets/pullrequests"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({ "values": values })),
        )
        .mount(server)
        .await;
}

fn pr_reviewed_by_wenyi() -> serde_json::Value {
    serde_json::json!({
        "id": 7,
        "title": "fix the thing",
        "state": "OPEN",
        "author": { "uuid": "{sean}", "nickname": "sean", "display_name": "Sean B" },
        "source": { "branch": { "name": "feature/a" } },
        "destination": { "branch": { "name": "main" } },
        "links": { "html": { "href": "https://bitbucket.org/acme/widgets/pull-requests/7" } },
        "reviewers": [{ "uuid": "{w}", "display_name": "Wenyi Ou" }],
        "participants": [
            { "role": "REVIEWER", "state": null, "user": { "uuid": "{w}", "display_name": "Wenyi Ou" } }
        ]
    })
}

/// Members 403s, but the permissions-config pool still carries the match — the
/// command must succeed, and the incomplete-pool warning must NOT appear on
/// stderr, because resolution didn't fail. Also pins that `--json` stdout stays
/// pure: nothing (warning or otherwise) leaks into it.
#[tokio::test]
async fn a_successful_resolution_via_permissions_config_does_not_warn() {
    let server = MockServer::start().await;
    mount_members_403(&server).await;
    mount_permissions_config(
        &server,
        serde_json::json!([{ "user": { "uuid": "{w}", "display_name": "Wenyi Ou", "nickname": "wenyi" } }]),
    )
    .await;
    mount_default_reviewers(&server, serde_json::json!([])).await;
    mount_list(&server, serde_json::json!([pr_reviewed_by_wenyi()])).await;

    let out = bb(&server)
        .args(["pr", "list", "--reviewer", "wenyi", "--json"])
        .output()
        .unwrap();

    assert!(
        out.status.success(),
        "expected success, stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("incomplete"),
        "warning fired on the success path: {stderr}"
    );

    // stdout must stay pure JSON — the warning must never leak into it.
    let value: serde_json::Value = serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| panic!("stdout was not pure JSON ({e}): {:?}", out.stdout));
    assert_eq!(value[0]["id"], 7);
}

/// Members 403s and nothing matches — resolution must fail with the ordinary
/// "no user matching" message, AND warn on stderr that the pool may be
/// incomplete, pointing at the `{uuid}` escape hatch.
#[tokio::test]
async fn a_failed_resolution_after_a_members_403_warns_on_stderr() {
    let server = MockServer::start().await;
    mount_members_403(&server).await;
    mount_permissions_config(&server, serde_json::json!([])).await;
    mount_default_reviewers(&server, serde_json::json!([])).await;

    let out = bb(&server)
        .args(["pr", "list", "--reviewer", "nobody", "--json"])
        .output()
        .unwrap();

    assert_eq!(out.status.code(), Some(1), "{:?}", out);

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("nobody"),
        "error should name the query: {stderr}"
    );
    assert!(
        stderr.contains("{uuid}"),
        "escape hatch missing from warning/error: {stderr}"
    );
    assert!(
        stderr.contains("some user lists could not be read"),
        "incomplete-pool warning missing or reworded: {stderr}"
    );
}
