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
    let mut cmd = suggest_human(server);
    cmd.arg("--json");
    cmd
}

fn suggest_human(server: &MockServer) -> Command {
    let mut cmd = bb(server);
    cmd.arg("pr")
        .arg("reviewers")
        .arg("suggest")
        .arg("--acknowledge-private-data");
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

async fn mount_one_maintained_file(server: &MockServer) {
    mount_existing_context(server, "acme/widgets").await;
    mount_pool(
        server,
        serde_json::json!([{ "user": { "uuid": "{dana}", "display_name": "Dana" } }]),
    )
    .await;
    for path in ["src/lib.rs", "old/name.rs"] {
        mount_target_history(
            server,
            "acme/widgets",
            "tgt-sha",
            path,
            serde_json::json!([commit("{dana}", "Dana", "m1", "2026-08-20T10:00:00+00:00")]),
            200,
        )
        .await;
        mount_source_history(
            server,
            "acme/widgets",
            "srcsha",
            path,
            serde_json::json!([]),
            200,
        )
        .await;
    }
}

/// The human view is what a person reads before choosing reviewers, and it is
/// where the claims the report keeps apart have to stay apart.
#[tokio::test]
async fn human_output_names_the_subject_scan_and_the_verdict() {
    let server = MockServer::start().await;
    mount_one_maintained_file(&server).await;

    let out = suggest_human(&server).args(["--pr", "7"]).output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("reviewer suggestions"), "{stdout}");
    assert!(stdout.contains("#7"), "{stdout}");
    assert!(stdout.contains("feature/x"), "{stdout}");
    assert!(stdout.contains("main"), "{stdout}");
    assert!(stdout.contains("files 2 scanned"), "{stdout}");
    assert!(stdout.contains("Dana"), "{stdout}");
    assert!(stdout.contains("CAN REVIEW"), "{stdout}");
    assert!(stdout.contains("yes"), "{stdout}");
}

/// A fork states both repositories, because a reader who does not know the
/// history came from elsewhere will misread the ranking.
#[tokio::test]
async fn human_output_names_both_repositories_for_a_fork() {
    let server = MockServer::start().await;
    mount_existing_context(&server, "contrib/widgets").await;
    mount_pool(&server, serde_json::json!([])).await;
    for path in ["src/lib.rs", "old/name.rs"] {
        mount_target_history(
            &server,
            "acme/widgets",
            "tgt-sha",
            path,
            serde_json::json!([]),
            200,
        )
        .await;
        mount_source_history(
            &server,
            "contrib/widgets",
            "srcsha",
            path,
            serde_json::json!([]),
            200,
        )
        .await;
    }

    let out = suggest_human(&server).args(["--pr", "7"]).output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("contrib/widgets"), "{stdout}");
    assert!(stdout.contains("acme/widgets"), "{stdout}");
    assert!(stdout.contains("no suggestions"), "{stdout}");
}

/// A prospective pair is not a pull request, so the header says what it is.
#[tokio::test]
async fn human_output_names_a_prospective_pair() {
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
        serde_json::json!([]),
        200,
    )
    .await;
    mount_source_history(
        &server,
        "acme/widgets",
        "srcsha",
        "src/one.rs",
        serde_json::json!([]),
        200,
    )
    .await;

    let out = suggest_human(&server)
        .args(["main", "feature/x"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("feature/x"), "{stdout}");
    assert!(stdout.contains("main"), "{stdout}");
    assert!(stdout.contains("no suggestions"), "{stdout}");
}

/// Partial history is a warning, not a footnote: a truncated read presented as
/// exhaustive is how a real maintainer gets missed without anyone noticing.
#[tokio::test]
async fn human_output_warns_when_the_history_is_partial() {
    let server = MockServer::start().await;
    mount_existing_context(&server, "acme/widgets").await;
    mount_pool(&server, serde_json::json!([])).await;
    // One path reads clean, one is unreadable: partial, but not empty.
    mount_target_history(
        &server,
        "acme/widgets",
        "tgt-sha",
        "src/lib.rs",
        serde_json::json!([commit("{dana}", "Dana", "m1", "2026-08-20T10:00:00+00:00")]),
        200,
    )
    .await;
    mount_source_history(
        &server,
        "acme/widgets",
        "srcsha",
        "src/lib.rs",
        serde_json::json!([]),
        200,
    )
    .await;
    mount_target_history(
        &server,
        "acme/widgets",
        "tgt-sha",
        "old/name.rs",
        serde_json::json!([]),
        404,
    )
    .await;
    mount_source_history(
        &server,
        "acme/widgets",
        "srcsha",
        "old/name.rs",
        serde_json::json!([]),
        404,
    )
    .await;

    let out = suggest_human(&server).args(["--pr", "7"]).output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("partial"), "{stderr}");
    // The per-path reason is named, not swallowed.
    assert!(stderr.contains("old/name.rs"), "{stderr}");
}

/// Every argument contract is settled before a single request goes out, so a bad
/// invocation cannot half-read a repository.
///
/// `--pr` against a positional target is rejected by clap before `run` is
/// reached; the guard inside `run` covers library callers, and is tested there.
#[tokio::test]
async fn the_argument_contracts_are_checked_before_any_request() {
    let cases: Vec<(Vec<&str>, &str)> = vec![
        (vec!["--pr", "7", "--since", "nonsense"], "invalid --since"),
        (
            vec!["--since", "0d", "main", "feature/x"],
            "--since must be between",
        ),
        (vec!["  "], "a target branch is required"),
        (vec!["main", "  "], "source branch cannot be empty"),
        (vec!["main", "main"], "source and target are both"),
    ];
    for (args, expected) in cases {
        let server = MockServer::start().await;
        let out = suggest(&server).args(&args).output().unwrap();
        assert!(!out.status.success(), "{args:?} was accepted");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(stderr.contains(expected), "{args:?}: {stderr}");
        assert!(
            server
                .received_requests()
                .await
                .unwrap_or_default()
                .is_empty(),
            "{args:?} made a request before failing: {stderr}"
        );
    }
}

/// A source repository Bitbucket will not name is not something to guess at: the
/// old fallback read the target's history and called it the fork's.
#[tokio::test]
async fn an_unreadable_source_repository_is_refused() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repositories/acme/widgets/pullrequests/7"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": 7,
            "author": { "uuid": "{author}", "display_name": "Author" },
            "source": {
                "branch": { "name": "feature/x" },
                "commit": { "hash": "srcsha" },
                "repository": { "full_name": "not a slug" }
            },
            "destination": {
                "branch": { "name": "main" },
                "commit": { "hash": "tgt-sha" },
                "repository": { "full_name": "acme/widgets" }
            }
        })))
        .mount(&server)
        .await;

    let out = suggest(&server).args(["--pr", "7"]).output().unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("source repository"), "{stderr}");
}

/// Without a pull request there is no endpoint to carry a revision, so the
/// branches are resolved once here. An empty branch has no history to speak of.
#[tokio::test]
async fn an_empty_branch_has_no_history_to_suggest_from() {
    let server = MockServer::start().await;
    mount_revision(&server, "feature%2Fx", "srcsha").await;
    Mock::given(method("GET"))
        .and(path(
            "/repositories/acme/widgets/diffstat/feature%2Fx..main",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "values": [{ "new": { "path": "src/one.rs" } }]
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repositories/acme/widgets/commits/main"))
        .and(query_param("pagelen", "1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "values": [] })))
        .mount(&server)
        .await;

    let out = suggest(&server)
        .args(["main", "feature/x"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("no commits"), "{stderr}");
}

/// An unreadable pool must not discard the ranking; eligibility just becomes
/// unknown rather than a verdict nobody earned.
#[tokio::test]
async fn an_unreadable_user_pool_leaves_eligibility_unknown() {
    let server = MockServer::start().await;
    mount_existing_context(&server, "acme/widgets").await;
    // Every pool endpoint fails hard, so no pool can be loaded at all.
    for endpoint in [
        "/workspaces/acme/members",
        "/repositories/acme/widgets/permissions-config/users",
        "/repositories/acme/widgets/effective-default-reviewers",
    ] {
        Mock::given(method("GET"))
            .and(path(endpoint))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;
    }
    for file in ["src/lib.rs", "old/name.rs"] {
        mount_target_history(
            &server,
            "acme/widgets",
            "tgt-sha",
            file,
            serde_json::json!([commit("{dana}", "Dana", "m1", "2026-08-20T10:00:00+00:00")]),
            200,
        )
        .await;
        mount_source_history(
            &server,
            "acme/widgets",
            "srcsha",
            file,
            serde_json::json!([]),
            200,
        )
        .await;
    }

    let out = suggest(&server).args(["--pr", "7"]).output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(value["suggestions"][0]["name"], "Dana");
    assert_eq!(value["suggestions"][0]["eligibility"], "unknown");
}

/// A commit the report cannot attribute is skipped, not guessed at: no linked
/// Bitbucket user, no uuid, an unparseable date, or one older than the window.
/// None of those may become a suggestion, and none may abort the run.
#[tokio::test]
async fn a_commit_that_cannot_be_attributed_is_skipped() {
    let server = MockServer::start().await;
    mount_existing_context(&server, "acme/widgets").await;
    mount_pool(&server, serde_json::json!([])).await;
    let skip = serde_json::json!([
        { "hash": "raw-only", "date": "2026-08-20T10:00:00+00:00", "author": { "raw": "Someone" } },
        { "hash": "no-uuid", "date": "2026-08-20T10:00:00+00:00", "author": { "user": { "display_name": "Nameless" } } },
        { "hash": "bad-date", "author": { "user": { "uuid": "{x}", "display_name": "Undated" } } },
        { "hash": "too-old", "date": "2000-01-01T00:00:00+00:00", "author": { "user": { "uuid": "{y}", "display_name": "Ancient" } } }
    ]);
    for path in ["src/lib.rs", "old/name.rs"] {
        mount_target_history(&server, "acme/widgets", "tgt-sha", path, skip.clone(), 200).await;
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
    assert!(
        out.status.success(),
        "unattributable commits must not fail the run: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(value["suggestions"], serde_json::json!([]));
}

/// Without `--source` the source branch is the current checkout's, which is the
/// workflow a developer runs from a feature branch.
#[tokio::test]
async fn an_explicit_target_uses_the_checkout_branch_by_default() {
    let project = tempfile::tempdir().unwrap();
    for args in [
        vec!["init"],
        vec!["symbolic-ref", "HEAD", "refs/heads/feature/checkout"],
        vec![
            "remote",
            "add",
            "origin",
            "https://bitbucket.org/acme/widgets",
        ],
    ] {
        std::process::Command::new("git")
            .args(&args)
            .current_dir(project.path())
            .output()
            .unwrap();
    }
    let server = MockServer::start().await;
    mount_revision(&server, "feature%2Fcheckout", "srcsha").await;
    mount_revision(&server, "main", "tgtsha").await;
    mount_current_user(&server).await;
    mount_pool(&server, serde_json::json!([])).await;
    Mock::given(method("GET"))
        .and(path(
            "/repositories/acme/widgets/diffstat/feature%2Fcheckout..main",
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
        serde_json::json!([]),
        200,
    )
    .await;
    mount_source_history(
        &server,
        "acme/widgets",
        "srcsha",
        "src/one.rs",
        serde_json::json!([]),
        200,
    )
    .await;

    let out = suggest(&server)
        .current_dir(project.path())
        .args(["main"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(value["prospective"]["source"], "feature/checkout");
}

/// The prospective flow excludes the future author. If the account cannot be
/// read, the command still runs — it just cannot drop anyone, and says so by
/// leaving the field it would have filtered on empty.
#[tokio::test]
async fn a_future_author_lookup_failure_does_not_stop_the_report() {
    for status in [404u16, 403] {
        let server = MockServer::start().await;
        mount_revision(&server, "feature%2Fx", "srcsha").await;
        mount_revision(&server, "main", "tgtsha").await;
        Mock::given(method("GET"))
            .and(path("/user"))
            .respond_with(ResponseTemplate::new(status))
            .mount(&server)
            .await;
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
            serde_json::json!([commit(
                "{other}",
                "Other",
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
            "src/one.rs",
            serde_json::json!([]),
            200,
        )
        .await;

        let out = suggest(&server)
            .args(["main", "feature/x"])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{status} on /user should not stop the report: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let value: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(value["suggestions"][0]["name"], "Other");
    }
}

/// An authentication failure is never a partial answer: it stops the run.
#[tokio::test]
async fn an_expired_token_stops_the_report() {
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
            401,
        )
        .await;
    }

    let out = suggest(&server).args(["--pr", "7"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2), "expected the auth exit code");
    assert!(String::from_utf8_lossy(&out.stderr).contains("not authenticated"));
}

/// Bitbucket omits the repository on an endpoint that is the same repository
/// the request was made against. Falling back to it is correct here — but only
/// because the caller passed the repository the request was made to.
#[tokio::test]
async fn an_endpoint_without_a_repository_falls_back_to_the_requested_one() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repositories/acme/widgets/pullrequests/7"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": 7,
            "author": { "uuid": "{author}", "display_name": "Author" },
            "source": {
                "branch": { "name": "feature/x" },
                "commit": { "hash": "srcsha" }
            },
            "destination": {
                "branch": { "name": "main" },
                "commit": { "hash": "tgt-sha" }
            }
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repositories/acme/widgets/pullrequests/7/diffstat"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "values": [{ "new": { "path": "src/one.rs" } }]
        })))
        .mount(&server)
        .await;
    mount_pool(&server, serde_json::json!([])).await;
    mount_target_history(
        &server,
        "acme/widgets",
        "tgt-sha",
        "src/one.rs",
        serde_json::json!([commit("{dana}", "Dana", "m1", "2026-08-20T10:00:00+00:00")]),
        200,
    )
    .await;
    mount_source_history(
        &server,
        "acme/widgets",
        "srcsha",
        "src/one.rs",
        serde_json::json!([]),
        200,
    )
    .await;

    let out = suggest(&server).args(["--pr", "7"]).output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(value["pull_request"]["source_repository"], "acme/widgets");
    assert_eq!(value["suggestions"][0]["name"], "Dana");
}

/// A commit author's display name can arrive on a later commit than the one that
/// first introduced them to the report. The name must not be lost.
#[tokio::test]
async fn a_display_name_arriving_later_is_still_reported() {
    let server = MockServer::start().await;
    mount_existing_context(&server, "acme/widgets").await;
    mount_pool(&server, serde_json::json!([])).await;
    let values = serde_json::json!([
        { "hash": "m1", "date": "2026-08-20T10:00:00+00:00",
          "author": { "user": { "uuid": "{dana}" } } },
        { "hash": "m2", "date": "2026-08-21T10:00:00+00:00",
          "author": { "user": { "uuid": "{dana}", "display_name": "Dana Fischer" } } }
    ]);
    for path in ["src/lib.rs", "old/name.rs"] {
        mount_target_history(
            &server,
            "acme/widgets",
            "tgt-sha",
            path,
            values.clone(),
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
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(value["suggestions"][0]["name"], "Dana Fischer");
}

/// "Can review" has three answers, and a human reader must be able to tell
/// "no" from "nobody could find out". Here one pool is refused and the person
/// is in none of the readable ones, so the answer is genuinely unknowable.
#[tokio::test]
async fn the_human_table_says_unknown_when_no_pool_could_be_read() {
    let server = MockServer::start().await;
    mount_existing_context(&server, "acme/widgets").await;
    // One pool refused, the rest readable: the person is present but the
    // answer is not knowable, so the table must not claim a yes or a no.
    Mock::given(method("GET"))
        .and(path("/workspaces/acme/members"))
        .respond_with(ResponseTemplate::new(403))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repositories/acme/widgets/permissions-config/users"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "values": []
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(
            "/repositories/acme/widgets/effective-default-reviewers",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "values": [] })))
        .mount(&server)
        .await;
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

    let out = suggest_human(&server).args(["--pr", "7"]).output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("unknown"), "{stdout}");
}

/// A server error on the account lookup is neither "not the author" nor a
/// partial answer; it is a failure, and it must not be read as either.
#[tokio::test]
async fn a_server_error_on_the_account_lookup_stops_the_prospective_report() {
    let server = MockServer::start().await;
    mount_revision(&server, "feature%2Fx", "srcsha").await;
    mount_revision(&server, "main", "tgtsha").await;
    Mock::given(method("GET"))
        .and(path("/user"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(
            "/repositories/acme/widgets/diffstat/feature%2Fx..main",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "values": [{ "new": { "path": "src/one.rs" } }]
        })))
        .mount(&server)
        .await;

    let out = suggest(&server)
        .args(["main", "feature/x"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("500"));
}
