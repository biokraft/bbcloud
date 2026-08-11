#![allow(clippy::unwrap_used)] // test code is exempt from the unwrap/expect ban

use assert_cmd::Command;
use wiremock::matchers::{method, path, query_param};
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

/// A pull request with three reviewers in three different states.
fn pr_with_reviewers() -> serde_json::Value {
    serde_json::json!({
        "id": 7,
        "title": "fix the thing",
        "state": "OPEN",
        "author": { "uuid": "{sean}", "nickname": "sean", "display_name": "Sean B" },
        "source": { "branch": { "name": "feature/a" } },
        "destination": { "branch": { "name": "main" } },
        "links": { "html": { "href": "https://bitbucket.org/acme/widgets/pull-requests/7" } },
        "reviewers": [
            { "uuid": "{a}", "display_name": "Ana" },
            { "uuid": "{b}", "display_name": "Bo" },
            { "uuid": "{c}", "display_name": "Cy" },
            { "uuid": "{d}", "display_name": "Dee" }
        ],
        "participants": [
            { "role": "REVIEWER", "state": "approved", "user": { "uuid": "{a}", "display_name": "Ana" } },
            { "role": "REVIEWER", "state": "changes_requested", "user": { "uuid": "{b}", "display_name": "Bo" } },
            { "role": "REVIEWER", "state": null, "user": { "uuid": "{c}", "display_name": "Cy" } },
            { "role": "PARTICIPANT", "state": "approved", "user": { "uuid": "{z}", "display_name": "Zed" } }
        ]
    })
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

/// The whole point of the feature: without this parameter Bitbucket returns a
/// reduced pull-request object with no reviewers, participants or draft flag, and
/// the reviewer column is silently empty.
#[tokio::test]
async fn list_requests_the_reviewer_fields() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repositories/acme/widgets/pullrequests"))
        .and(query_param(
            "fields",
            "+values.reviewers,+values.participants,+values.draft",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "values": [] })))
        .mount(&server)
        .await;

    bb(&server).args(["pr", "list"]).assert().success();
}

#[tokio::test]
async fn list_marks_each_reviewer_with_their_state() {
    let server = MockServer::start().await;
    mount_list(&server, serde_json::json!([pr_with_reviewers()])).await;

    let out = bb(&server).args(["pr", "list"]).output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("Ana ✓"), "{text}");
    assert!(text.contains("Bo ✗"), "{text}");
    assert!(text.contains("Cy ·"), "{text}");
    // Tagged but never seen by the api's participant list.
    assert!(text.contains("Dee ·"), "{text}");
    // A commenter is not a reviewer.
    assert!(
        !text.contains("Zed"),
        "plain participant rendered as reviewer: {text}"
    );
}

#[tokio::test]
async fn list_shows_the_pr_state_column() {
    let server = MockServer::start().await;
    let mut draft = pr_with_reviewers();
    draft["draft"] = serde_json::json!(true);
    let mut declined = pr_with_reviewers();
    declined["id"] = serde_json::json!(8);
    declined["state"] = serde_json::json!("DECLINED");
    mount_list(&server, serde_json::json!([draft, declined])).await;

    let out = bb(&server)
        .args(["pr", "list", "--state", "all"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("STATE"), "{text}");
    assert!(text.contains("Draft"), "{text}");
    assert!(text.contains("Declined"), "{text}");
}

#[tokio::test]
async fn list_json_emits_structured_reviewers_and_no_approvals_key() {
    let server = MockServer::start().await;
    mount_list(&server, serde_json::json!([pr_with_reviewers()])).await;

    let out = bb(&server).args(["pr", "list", "--json"]).output().unwrap();
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(value[0]["id"], 7);
    assert_eq!(value[0]["state"], "OPEN");
    assert_eq!(value[0]["draft"], false);
    assert_eq!(value[0]["reviewers"][0]["name"], "Ana");
    assert_eq!(value[0]["reviewers"][0]["uuid"], "{a}");
    assert_eq!(value[0]["reviewers"][0]["state"], "approved");
    assert_eq!(value[0]["reviewers"][1]["state"], "changes_requested");
    assert_eq!(value[0]["reviewers"][2]["state"], "pending");
    assert!(
        value[0].get("approvals").is_none(),
        "approvals key should be gone: {value}"
    );
}

#[tokio::test]
async fn list_json_on_zero_rows_is_a_pure_empty_array() {
    let server = MockServer::start().await;
    mount_list(&server, serde_json::json!([])).await;

    let out = bb(&server).args(["pr", "list", "--json"]).output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    let value: serde_json::Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("stdout did not parse as JSON: {e}\nstdout: {stdout}"));
    assert_eq!(value, serde_json::json!([]));
}

#[tokio::test]
async fn list_still_filters_by_destination_branch() {
    let server = MockServer::start().await;
    let mut other = pr_with_reviewers();
    other["id"] = serde_json::json!(8);
    other["source"] = serde_json::json!({ "branch": { "name": "feature/b" } });
    other["destination"] = serde_json::json!({ "branch": { "name": "develop" } });
    mount_list(&server, serde_json::json!([pr_with_reviewers(), other])).await;

    let out = bb(&server)
        .args(["pr", "list", "develop"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("feature/b"), "{text}");
    assert!(
        !text.contains("feature/a"),
        "destination filter not applied: {text}"
    );
}

#[tokio::test]
async fn list_state_all_asks_the_api_for_every_state() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repositories/acme/widgets/pullrequests"))
        .and(query_param("state", "OPEN,MERGED,DECLINED,SUPERSEDED"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "values": [] })))
        .mount(&server)
        .await;

    bb(&server)
        .args(["pr", "list", "--state", "all"])
        .assert()
        .success();
}

/// Two pull requests: 7 has Ana approving and Dee pending, 8 has only Dee.
fn pr_pair() -> serde_json::Value {
    let mut second = pr_with_reviewers();
    second["id"] = serde_json::json!(8);
    second["title"] = serde_json::json!("other thing");
    second["source"] = serde_json::json!({ "branch": { "name": "feature/b" } });
    second["author"] =
        serde_json::json!({ "uuid": "{a}", "nickname": "ana", "display_name": "Ana" });
    second["reviewers"] = serde_json::json!([{ "uuid": "{d}", "display_name": "Dee" }]);
    second["participants"] = serde_json::json!([
        { "role": "REVIEWER", "state": null, "user": { "uuid": "{d}", "display_name": "Dee" } }
    ]);
    serde_json::json!([pr_with_reviewers(), second])
}

async fn mount_members(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/workspaces/acme/members"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "values": [
                { "user": { "uuid": "{a}", "display_name": "Ana", "nickname": "ana" } },
                { "user": { "uuid": "{b}", "display_name": "Bo", "nickname": "bo" } },
                { "user": { "uuid": "{d}", "display_name": "Dee", "nickname": "dee" } },
                { "user": { "uuid": "{sean}", "display_name": "Sean B", "nickname": "sean" } }
            ]
        })))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repositories/acme/widgets/default-reviewers"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "values": [] })))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repositories/acme/widgets/permissions-config/users"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "values": [] })))
        .mount(server)
        .await;
}

/// `@me` and the review-state filters need to know who the token belongs to.
async fn mount_me(server: &MockServer, uuid: &str, name: &str) {
    Mock::given(method("GET"))
        .and(path("/user"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "uuid": uuid,
            "display_name": name,
            "nickname": name.to_lowercase()
        })))
        .mount(server)
        .await;
}

#[tokio::test]
async fn state_draft_keeps_only_drafts() {
    let server = MockServer::start().await;
    let mut draft = pr_with_reviewers();
    draft["draft"] = serde_json::json!(true);
    let mut plain = pr_with_reviewers();
    plain["id"] = serde_json::json!(8);
    plain["title"] = serde_json::json!("not a draft");
    mount_list(&server, serde_json::json!([draft, plain])).await;

    let out = bb(&server)
        .args(["pr", "list", "--state", "draft"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("fix the thing"), "{text}");
    assert!(
        !text.contains("not a draft"),
        "draft filter not applied: {text}"
    );
}

/// `--state draft` still has to ask bitbucket for OPEN: DRAFT is not an api state.
#[tokio::test]
async fn state_draft_requests_open_from_the_api() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repositories/acme/widgets/pullrequests"))
        .and(query_param("state", "OPEN"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "values": [] })))
        .mount(&server)
        .await;

    bb(&server)
        .args(["pr", "list", "--state", "draft"])
        .assert()
        .success();
}

#[tokio::test]
async fn reviewer_filter_keeps_only_prs_that_person_reviews() {
    let server = MockServer::start().await;
    mount_list(&server, pr_pair()).await;
    mount_members(&server).await;

    let out = bb(&server)
        .args(["pr", "list", "--reviewer", "bo"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("fix the thing"), "{text}");
    assert!(
        !text.contains("other thing"),
        "reviewer filter not applied: {text}"
    );
}

#[tokio::test]
async fn author_filter_keeps_only_that_authors_prs() {
    let server = MockServer::start().await;
    mount_list(&server, pr_pair()).await;
    mount_members(&server).await;

    let out = bb(&server)
        .args(["pr", "list", "--author", "ana"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("other thing"), "{text}");
    assert!(
        !text.contains("fix the thing"),
        "author filter not applied: {text}"
    );
}

#[tokio::test]
async fn author_me_uses_the_authenticated_account() {
    let server = MockServer::start().await;
    mount_list(&server, pr_pair()).await;
    mount_me(&server, "{a}", "Ana").await;

    let out = bb(&server)
        .args(["pr", "list", "--author", "@me"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("other thing"), "{text}");
    assert!(!text.contains("fix the thing"), "@me not resolved: {text}");
}

#[tokio::test]
async fn needs_my_review_keeps_prs_where_i_have_not_approved() {
    let server = MockServer::start().await;
    // I am Bo, who requested changes on 7 and is not a reviewer on 8.
    mount_list(&server, pr_pair()).await;
    mount_me(&server, "{b}", "Bo").await;

    let out = bb(&server)
        .args(["pr", "list", "--needs-my-review"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("fix the thing"), "{text}");
    assert!(
        !text.contains("other thing"),
        "not-a-reviewer pr kept: {text}"
    );
}

#[tokio::test]
async fn needs_my_review_drops_prs_i_already_approved() {
    let server = MockServer::start().await;
    // I am Ana, who approved 7 and does not review 8.
    mount_list(&server, pr_pair()).await;
    mount_me(&server, "{a}", "Ana").await;

    let out = bb(&server)
        .args(["pr", "list", "--needs-my-review"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        !text.contains("fix the thing"),
        "approved pr still listed: {text}"
    );
    assert!(!text.contains("other thing"), "{text}");
}

#[tokio::test]
async fn review_state_approved_keeps_only_what_i_approved() {
    let server = MockServer::start().await;
    mount_list(&server, pr_pair()).await;
    mount_me(&server, "{a}", "Ana").await;

    let out = bb(&server)
        .args(["pr", "list", "--review-state", "approved"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("fix the thing"), "{text}");
    assert!(!text.contains("other thing"), "{text}");
}

#[tokio::test]
async fn review_state_changes_requested_matches_my_rejection() {
    let server = MockServer::start().await;
    mount_list(&server, pr_pair()).await;
    mount_me(&server, "{b}", "Bo").await;

    let out = bb(&server)
        .args(["pr", "list", "--review-state", "changes-requested"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("fix the thing"), "{text}");
    assert!(!text.contains("other thing"), "{text}");
}

#[tokio::test]
async fn review_state_pending_matches_an_unreviewed_tag() {
    let server = MockServer::start().await;
    mount_list(&server, pr_pair()).await;
    // Cy is tagged on 7 but has no participant entry, so their state is Pending.
    // Cy is not tagged on 8 at all.
    mount_me(&server, "{c}", "Cy").await;

    let out = bb(&server)
        .args(["pr", "list", "--review-state", "pending"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("fix the thing"), "{text}");
    assert!(!text.contains("other thing"), "{text}");
}

/// `--author @me` and `--needs-my-review` both need "who am I"; they must share one
/// `GET /user` call rather than each fetching it independently.
#[tokio::test]
async fn author_me_and_needs_my_review_share_a_single_user_fetch() {
    let server = MockServer::start().await;
    mount_list(&server, pr_pair()).await;
    Mock::given(method("GET"))
        .and(path("/user"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "uuid": "{sean}",
            "display_name": "Sean B",
            "nickname": "sean"
        })))
        .expect(1)
        .mount(&server)
        .await;

    bb(&server)
        .args(["pr", "list", "--author", "@me", "--needs-my-review"])
        .assert()
        .success();
}

/// Filters must AND, not OR.
#[tokio::test]
async fn two_filters_intersect() {
    let server = MockServer::start().await;
    mount_list(&server, pr_pair()).await;
    mount_members(&server).await;

    // Dee reviews both; only 8 is authored by Ana.
    let out = bb(&server)
        .args(["pr", "list", "--reviewer", "dee", "--author", "ana"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("other thing"), "{text}");
    assert!(
        !text.contains("fix the thing"),
        "filters ORed instead of ANDed: {text}"
    );
}

#[tokio::test]
async fn an_invalid_review_state_is_rejected_before_any_request() {
    let server = MockServer::start().await;

    bb(&server)
        .args(["pr", "list", "--review-state", "nonsense"])
        .assert()
        .failure();
    assert!(
        server
            .received_requests()
            .await
            .unwrap_or_default()
            .is_empty(),
        "clap should reject the value before any http call"
    );
}

#[tokio::test]
async fn a_filter_matching_nothing_prints_an_empty_json_array() {
    let server = MockServer::start().await;
    mount_list(&server, pr_pair()).await;
    mount_members(&server).await;

    let out = bb(&server)
        .args([
            "pr",
            "list",
            "--reviewer",
            "bo",
            "--author",
            "ana",
            "--json",
        ])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let value: serde_json::Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("stdout did not parse as JSON: {e}\nstdout: {stdout}"));
    assert_eq!(value, serde_json::json!([]));
}
