#![allow(clippy::unwrap_used)]

use assert_cmd::Command;
use wiremock::matchers::{method, path, query_param};
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

fn pr_body() -> serde_json::Value {
    serde_json::json!({
        "id": 7,
        "author": { "uuid": "{author}", "display_name": "Author" },
        "reviewers": [{ "uuid": "{reviewer}", "display_name": "Reviewer" }],
        "source": {
            "branch": { "name": "feature/x" },
            "repository": { "full_name": "acme/widgets" }
        },
        "destination": {
            "branch": { "name": "main" },
            "repository": { "full_name": "acme/widgets" }
        }
    })
}

async fn mount_existing_context(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/repositories/acme/widgets/pullrequests/7"))
        .respond_with(ResponseTemplate::new(200).set_body_json(pr_body()))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repositories/acme/widgets/pullrequests/7/diffstat"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "values": [
                { "new": { "path": "src/lib.rs" } },
                { "old": { "path": "README.md" } }
            ]
        })))
        .mount(server)
        .await;
}

async fn mount_history(
    server: &MockServer,
    path_name: &str,
    values: serde_json::Value,
    status: u16,
) {
    Mock::given(method("GET"))
        .and(path("/repositories/acme/widgets/commits/feature%2Fx"))
        .and(query_param("path", path_name))
        .and(query_param("exclude", "main"))
        .respond_with(
            ResponseTemplate::new(status).set_body_json(serde_json::json!({ "values": values })),
        )
        .mount(server)
        .await;
}

#[tokio::test]
async fn existing_pr_suggestions_include_evidence_and_exclude_existing_people() {
    let server = MockServer::start().await;
    mount_existing_context(&server).await;
    mount_history(
        &server,
        "src/lib.rs",
        serde_json::json!([
            {
                "hash": "c1",
                "date": "2026-08-20T10:00:00+00:00",
                "author": { "user": { "uuid": "{dana}", "display_name": "Dana" } },
                "summary": { "raw": "change lib" }
            },
            {
                "hash": "c2",
                "date": "2026-08-21T10:00:00+00:00",
                "author": { "user": { "uuid": "{author}", "display_name": "Author" } }
            },
            {
                "hash": "c3",
                "date": "2026-08-21T11:00:00+00:00",
                "author": { "user": { "uuid": "{reviewer}", "display_name": "Reviewer" } }
            },
            {
                "hash": "c4",
                "date": "2026-08-21T12:00:00+00:00",
                "author": { "raw": "No Bitbucket user" }
            }
        ]),
        200,
    )
    .await;
    mount_history(
        &server,
        "README.md",
        serde_json::json!([{
            "hash": "c5",
            "date": "2026-08-22T10:00:00+00:00",
            "author": { "user": { "uuid": "{dana}", "display_name": "Dana" } }
        }]),
        200,
    )
    .await;

    let out = bb(&server)
        .args(["pr", "reviewers", "suggest", "--pr", "7", "--json"])
        .output()
        .unwrap();
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(value["pull_request"]["id"], 7);
    assert_eq!(value["files_scanned"], 2);
    assert_eq!(value["history_complete"], true);
    let suggestions = value["suggestions"].as_array().unwrap();
    assert_eq!(suggestions.len(), 1);
    assert_eq!(suggestions[0]["name"], "Dana");
    assert_eq!(suggestions[0]["commit_count"], 2);
    assert_eq!(
        suggestions[0]["files"],
        serde_json::json!(["README.md", "src/lib.rs"])
    );
}

#[tokio::test]
async fn prospective_suggestions_report_skipped_files_and_partial_history() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/repositories/acme/widgets/diffstat/feature%2Fx..main",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "values": [
                { "new": { "path": "src/one.rs" } },
                { "new": { "path": "src/two.rs" } },
                { "new": { "path": "src/three.rs" } }
            ]
        })))
        .mount(&server)
        .await;
    mount_history(
        &server,
        "src/one.rs",
        serde_json::json!([{
            "hash": "c1",
            "date": "2026-08-20T10:00:00+00:00",
            "author": { "user": { "uuid": "{one}", "display_name": "One" } }
        }]),
        200,
    )
    .await;
    mount_history(&server, "src/two.rs", serde_json::json!([]), 403).await;
    mount_history(&server, "src/three.rs", serde_json::json!([]), 403).await;

    let out = bb(&server)
        .args([
            "pr",
            "reviewers",
            "suggest",
            "main",
            "feature/x",
            "--file-limit",
            "2",
            "--json",
        ])
        .output()
        .unwrap();
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(value["prospective"]["source"], "feature/x");
    assert_eq!(value["prospective"]["target"], "main");
    assert_eq!(value["files_scanned"], 2);
    assert_eq!(value["files_skipped"], 1);
    assert_eq!(value["history_complete"], false);
    assert_eq!(value["errors"][0]["path"], "src/two.rs");
    assert_eq!(value["suggestions"][0]["name"], "One");
}

#[tokio::test]
async fn a_history_page_limited_to_one_hundred_is_reported_as_incomplete() {
    let server = MockServer::start().await;
    mount_existing_context(&server).await;
    let values: Vec<serde_json::Value> = (0..100)
        .map(|index| {
            serde_json::json!({
                "hash": format!("c{index}"),
                "date": "2026-08-20T10:00:00+00:00",
                "author": { "user": { "uuid": "{dana}", "display_name": "Dana" } }
            })
        })
        .collect();
    mount_history(&server, "src/lib.rs", serde_json::json!(values), 200).await;
    mount_history(&server, "README.md", serde_json::json!([]), 200).await;

    let out = bb(&server)
        .args(["pr", "reviewers", "suggest", "--pr", "7", "--json"])
        .output()
        .unwrap();
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(value["history_complete"], false);
}
