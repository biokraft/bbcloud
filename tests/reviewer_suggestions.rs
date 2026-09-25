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

fn suggest(server: &MockServer) -> Command {
    let mut cmd = bb(server);
    cmd.arg("pr")
        .arg("reviewers")
        .arg("suggest")
        .arg("--acknowledge-private-data")
        .arg("--json");
    cmd
}

fn commit(uuid: &str, name: &str, hash: &str, date: &str) -> serde_json::Value {
    serde_json::json!({
        "hash": hash,
        "date": date,
        "author": { "user": { "uuid": uuid, "display_name": name } }
    })
}

fn pr_body(source_repo: &str) -> serde_json::Value {
    serde_json::json!({
        "id": 7,
        "author": { "uuid": "{author}", "display_name": "Author" },
        "reviewers": [{ "uuid": "{reviewer}", "display_name": "Reviewer" }],
        "source": {
            "branch": { "name": "feature/x" },
            "commit": { "hash": "srcsha" },
            "repository": { "full_name": source_repo }
        },
        "destination": {
            "branch": { "name": "main" },
            "commit": { "hash": "tgt-sha" },
            "repository": { "full_name": "acme/widgets" }
        }
    })
}

async fn mount_existing_context(server: &MockServer, source_repo: &str) {
    Mock::given(method("GET"))
        .and(path("/repositories/acme/widgets/pullrequests/7"))
        .respond_with(ResponseTemplate::new(200).set_body_json(pr_body(source_repo)))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repositories/acme/widgets/pullrequests/7/diffstat"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "values": [
                { "new": { "path": "src/lib.rs" } },
                { "status": "renamed",
                  "old": { "path": "old/name.rs" },
                  "new": { "path": "src/lib.rs" } }
            ]
        })))
        .mount(server)
        .await;
}

/// The target branch's history, with nothing subtracted: this is the population
/// of people who actually maintain the changed files.
async fn mount_target_history(
    server: &MockServer,
    repository: &str,
    revision: &str,
    path_name: &str,
    values: serde_json::Value,
    status: u16,
) {
    Mock::given(method("GET"))
        .and(path(format!(
            "/repositories/{repository}/commits/{revision}"
        )))
        .and(query_param("path", path_name))
        .respond_with(
            ResponseTemplate::new(status).set_body_json(serde_json::json!({ "values": values })),
        )
        .mount(server)
        .await;
}

/// Source history minus the target: the people who worked on this branch and
/// are not already reachable from the target.
async fn mount_source_history(
    server: &MockServer,
    repository: &str,
    revision: &str,
    path_name: &str,
    values: serde_json::Value,
    status: u16,
) {
    Mock::given(method("GET"))
        .and(path(format!(
            "/repositories/{repository}/commits/{revision}"
        )))
        .and(query_param("path", path_name))
        .and(query_param("exclude", "main"))
        .respond_with(
            ResponseTemplate::new(status).set_body_json(serde_json::json!({ "values": values })),
        )
        .mount(server)
        .await;
}

async fn mount_pool(server: &MockServer, permissions: serde_json::Value) {
    Mock::given(method("GET"))
        .and(path("/workspaces/acme/members"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "values": [] })))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repositories/acme/widgets/permissions-config/users"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({ "values": permissions })),
        )
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path(
            "/repositories/acme/widgets/effective-default-reviewers",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "values": [] })))
        .mount(server)
        .await;
}

async fn mount_current_user(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/user"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "uuid": "{me}", "display_name": "Me"
        })))
        .mount(server)
        .await;
}

async fn mount_revision(server: &MockServer, revision: &str, hash: &str) {
    Mock::given(method("GET"))
        .and(path(format!(
            "/repositories/acme/widgets/commits/{revision}"
        )))
        .and(query_param("pagelen", "1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "values": [{ "hash": hash }]
        })))
        .mount(server)
        .await;
}

/// The maintainers of the file are the point of the command. Reading the source
/// branch with the target subtracted removes exactly the commits that are
/// already merged, which is how the original implementation found nobody.
#[tokio::test]
async fn suggestions_come_from_target_maintainers_and_source_coauthors() {
    let server = MockServer::start().await;
    mount_existing_context(&server, "acme/widgets").await;
    mount_pool(
        &server,
        serde_json::json!([{ "user": { "uuid": "{dana}", "display_name": "Dana" } }]),
    )
    .await;
    for path in ["src/lib.rs", "old/name.rs"] {
        mount_target_history(
            &server,
            "acme/widgets",
            "tgt-sha",
            path,
            serde_json::json!([
                commit("{dana}", "Dana", "m1", "2026-08-20T10:00:00+00:00"),
                commit("{author}", "Author", "m2", "2026-08-20T11:00:00+00:00"),
                commit("{reviewer}", "Reviewer", "m3", "2026-08-20T12:00:00+00:00"),
            ]),
            200,
        )
        .await;
        mount_source_history(
            &server,
            "acme/widgets",
            "srcsha",
            path,
            serde_json::json!([commit(
                "{coauthor}",
                "Coauthor",
                "s1",
                "2026-08-21T10:00:00+00:00"
            ),]),
            200,
        )
        .await;
    }

    let out = suggest(&server).args(["--pr", "7"]).output().unwrap();
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(value["pull_request"]["id"], 7);
    // The evidence is pinned to immutable revisions, not to moving branches.
    assert_eq!(value["pull_request"]["source_revision"], "srcsha");
    assert_eq!(value["pull_request"]["target_revision"], "tgt-sha");
    let suggestions = value["suggestions"].as_array().unwrap();
    let names: Vec<&str> = suggestions
        .iter()
        .map(|s| s["name"].as_str().unwrap())
        .collect();
    // The maintainer outranks the branch coauthor; the author and the existing
    // reviewer are not suggested to themselves.
    assert_eq!(names, vec!["Dana", "Coauthor"]);
    // One commit is one commit, however many of the changed files it touched.
    assert_eq!(suggestions[0]["target_commits"], 1);
    assert_eq!(suggestions[0]["files"].as_array().unwrap().len(), 2);
    assert_eq!(suggestions[1]["target_commits"], 0);
    assert_eq!(suggestions[1]["source_commits"], 1);
    // A rename is history under the old path too.
    assert!(suggestions[0]["files"]
        .as_array()
        .unwrap()
        .iter()
        .any(|f| f == "old/name.rs"));
    // Dana is in the repository's own permission configuration.
    assert_eq!(suggestions[0]["eligibility"], "true");
    // The pool was read in full and the coauthor is in none of it, so this is a
    // real negative rather than a missing answer.
    assert_eq!(suggestions[1]["eligibility"], "false");
}

#[tokio::test]
async fn history_read_from_an_unreadable_source_repository_is_fatal() {
    let server = MockServer::start().await;
    mount_existing_context(&server, "acme/widgets").await;
    mount_pool(&server, serde_json::json!([])).await;
    // A rate limit applies to the whole call, not to one file. Reporting it as
    // a per-path error and returning success is how a broken endpoint hides.
    mount_target_history(
        &server,
        "acme/widgets",
        "tgt-sha",
        "src/lib.rs",
        serde_json::json!([]),
        429,
    )
    .await;
    mount_target_history(
        &server,
        "acme/widgets",
        "tgt-sha",
        "old/name.rs",
        serde_json::json!([]),
        429,
    )
    .await;

    let out = suggest(&server).args(["--pr", "7"]).output().unwrap();
    assert!(!out.status.success());
    assert!(value_error_is_rate_limit(&out.stderr));
}

fn value_error_is_rate_limit(stderr: &[u8]) -> bool {
    String::from_utf8_lossy(stderr).contains("429")
}

/// Every path failing is a failure of the command, not an empty result.
#[tokio::test]
async fn no_readable_history_at_all_is_an_error() {
    let server = MockServer::start().await;
    mount_existing_context(&server, "acme/widgets").await;
    mount_pool(&server, serde_json::json!([])).await;
    for path in ["src/lib.rs", "old/name.rs"] {
        mount_target_history(
            &server,
            "acme/widgets",
            "tgt-sha",
            path,
            serde_json::json!([]),
            404,
        )
        .await;
        mount_source_history(
            &server,
            "acme/widgets",
            "srcsha",
            path,
            serde_json::json!([]),
            404,
        )
        .await;
    }

    let out = suggest(&server).args(["--pr", "7"]).output().unwrap();
    assert!(!out.status.success(), "expected a failure, got {out:?}");
    assert!(String::from_utf8_lossy(&out.stderr).contains("no file history"));
}

/// A fork's source branch does not contain the target's commits, so subtracting
/// the target there would read a population that does not exist.
#[tokio::test]
async fn a_fork_reads_both_repositories_and_subtracts_nothing() {
    let server = MockServer::start().await;
    mount_existing_context(&server, "contrib/widgets").await;
    mount_pool(
        &server,
        serde_json::json!([{ "user": { "uuid": "{dana}", "display_name": "Dana" } }]),
    )
    .await;
    for path in ["src/lib.rs", "old/name.rs"] {
        mount_target_history(
            &server,
            "acme/widgets",
            "tgt-sha",
            path,
            serde_json::json!([commit("{dana}", "Dana", "m1", "2026-08-20T10:00:00+00:00")]),
            200,
        )
        .await;
    }
    // No `exclude` matcher: a request carrying one would not match this mock
    // and the fork's own history would be silently missing from the report.
    Mock::given(method("GET"))
        .and(path("/repositories/contrib/widgets/commits/srcsha"))
        .and(query_param("path", "src/lib.rs"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "values": [commit("{forker}", "Forker", "f1", "2026-08-21T10:00:00+00:00")]
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repositories/contrib/widgets/commits/srcsha"))
        .and(query_param("path", "old/name.rs"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "values": [] })))
        .mount(&server)
        .await;

    let out = suggest(&server).args(["--pr", "7"]).output().unwrap();
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(
        value["pull_request"]["source_repository"],
        "contrib/widgets"
    );
    assert_eq!(value["pull_request"]["target_repository"], "acme/widgets");
    let names: Vec<&str> = value["suggestions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["name"].as_str().unwrap())
        .collect();
    assert!(
        names.contains(&"Forker"),
        "fork history was skipped: {names:?}"
    );
    assert!(names.contains(&"Dana"));
}

/// A complete pool that does not contain the person is a real negative, not an
/// unknown.
#[tokio::test]
async fn a_complete_pool_marks_an_absent_person_as_ineligible() {
    let server = MockServer::start().await;
    mount_existing_context(&server, "acme/widgets").await;
    mount_pool(&server, serde_json::json!([])).await;
    for path in ["src/lib.rs", "old/name.rs"] {
        mount_target_history(
            &server,
            "acme/widgets",
            "tgt-sha",
            path,
            serde_json::json!([commit(
                "{stranger}",
                "Stranger",
                "m1",
                "2026-08-20T10:00:00+00:00"
            )]),
            200,
        )
        .await;
        mount_source_history(
            &server,
            "acme/widgets",
            "srcsha",
            path,
            serde_json::json!([]),
            200,
        )
        .await;
    }

    let out = suggest(&server).args(["--pr", "7"]).output().unwrap();
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(value["suggestions"][0]["eligibility"], "false");
}

/// The person about to open the pull request is its author, and Bitbucket
/// rejects the author as a reviewer.
#[tokio::test]
async fn prospective_suggestions_never_offer_the_future_author() {
    let server = MockServer::start().await;
    mount_revision(&server, "feature%2Fx", "srcsha").await;
    mount_revision(&server, "main", "tgtsha").await;
    mount_current_user(&server).await;
    mount_pool(&server, serde_json::json!([])).await;
    Mock::given(method("GET"))
        .and(path(
            "/repositories/acme/widgets/diffstat/feature%2Fx..main",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "values": [{ "new": { "path": "src/one.rs" } }]
        })))
        .mount(&server)
        .await;
    mount_target_history(
        &server,
        "acme/widgets",
        "tgtsha",
        "src/one.rs",
        serde_json::json!([
            commit("{me}", "Me", "m1", "2026-08-20T10:00:00+00:00"),
            commit("{other}", "Other", "m2", "2026-08-20T11:00:00+00:00"),
        ]),
        200,
    )
    .await;
    mount_source_history(
        &server,
        "acme/widgets",
        "srcsha",
        "src/one.rs",
        serde_json::json!([commit("{me}", "Me", "s1", "2026-08-21T10:00:00+00:00")]),
        200,
    )
    .await;

    let out = suggest(&server)
        .args(["main", "feature/x"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(value["prospective"]["source_revision"], "srcsha");
    assert_eq!(value["prospective"]["target_revision"], "tgtsha");
    let names: Vec<&str> = value["suggestions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["Other"]);
}

#[tokio::test]
async fn a_page_limited_to_one_hundred_commits_is_reported_as_incomplete() {
    let server = MockServer::start().await;
    mount_existing_context(&server, "acme/widgets").await;
    mount_pool(&server, serde_json::json!([])).await;
    let values: Vec<serde_json::Value> = (0..100)
        .map(|index| {
            commit(
                "{dana}",
                "Dana",
                &format!("c{index}"),
                "2026-08-20T10:00:00+00:00",
            )
        })
        .collect();
    mount_target_history(
        &server,
        "acme/widgets",
        "tgt-sha",
        "src/lib.rs",
        serde_json::json!(values),
        200,
    )
    .await;
    mount_target_history(
        &server,
        "acme/widgets",
        "tgt-sha",
        "old/name.rs",
        serde_json::json!([]),
        200,
    )
    .await;
    for path in ["src/lib.rs", "old/name.rs"] {
        mount_source_history(
            &server,
            "acme/widgets",
            "srcsha",
            path,
            serde_json::json!([]),
            200,
        )
        .await;
    }

    let out = suggest(&server).args(["--pr", "7"]).output().unwrap();
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(value["history_complete"], false);
}

/// Reading history sends private file paths, colleague names, dates and account
/// ids to whoever is on the other end of the model. That is the user's call.
#[tokio::test]
async fn reading_history_needs_an_explicit_acknowledgement() {
    let server = MockServer::start().await;
    let out = bb(&server)
        .args(["pr", "reviewers", "suggest", "--pr", "7", "--json"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("--acknowledge-private-data"), "{stderr}");
    assert!(
        server
            .received_requests()
            .await
            .unwrap_or_default()
            .is_empty(),
        "no request may be made before the acknowledgement"
    );
}
