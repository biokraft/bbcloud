#![allow(clippy::unwrap_used)] // test code is exempt from the unwrap/expect ban

mod support;

use bb_cli::error::BbError;
use bb_cli::repo::RepoSlug;
use bb_cli::users::resolve_user;
use support::client_for;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn slug() -> RepoSlug {
    RepoSlug::parse("acme/widgets").unwrap()
}

async fn mount_members(server: &MockServer, members: serde_json::Value) {
    Mock::given(method("GET"))
        .and(path("/workspaces/acme/members"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({ "values": members })),
        )
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

/// A uuid is already exact, so resolution must not spend two api calls on it.
#[tokio::test]
async fn a_uuid_is_used_verbatim_without_any_lookup() {
    let server = MockServer::start().await;

    let user = resolve_user(&client_for(&server.uri()), &slug(), "{9a1b}", &[])
        .await
        .unwrap();
    assert_eq!(user.uuid.as_deref(), Some("{9a1b}"));
    assert!(
        server
            .received_requests()
            .await
            .unwrap_or_default()
            .is_empty(),
        "a uuid should not trigger a lookup"
    );
}

#[tokio::test]
async fn a_substring_of_the_display_name_resolves() {
    let server = MockServer::start().await;
    mount_members(
        &server,
        serde_json::json!([
            { "user": { "uuid": "{p}", "display_name": "Patrick Stein", "nickname": "patrick" } },
            { "user": { "uuid": "{r}", "display_name": "Raigon Doe", "nickname": "raigon" } }
        ]),
    )
    .await;
    mount_default_reviewers(&server, serde_json::json!([])).await;

    let user = resolve_user(&client_for(&server.uri()), &slug(), "patri", &[])
        .await
        .unwrap();
    assert_eq!(user.uuid.as_deref(), Some("{p}"));
}

#[tokio::test]
async fn an_ambiguous_query_errors_and_names_every_candidate() {
    let server = MockServer::start().await;
    mount_members(
        &server,
        serde_json::json!([
            { "user": { "uuid": "{1}", "display_name": "Ana Cruz" } },
            { "user": { "uuid": "{2}", "display_name": "Anastasia Ivanova" } }
        ]),
    )
    .await;
    mount_default_reviewers(&server, serde_json::json!([])).await;

    let err = resolve_user(&client_for(&server.uri()), &slug(), "ana", &[])
        .await
        .unwrap_err();
    match err {
        BbError::Config(message) => {
            assert!(message.contains("Ana Cruz"), "{message}");
            assert!(message.contains("Anastasia Ivanova"), "{message}");
            assert!(
                message.contains("uuid"),
                "no escape hatch offered: {message}"
            );
        }
        other => panic!("unexpected: {other:?}"),
    }
}

/// Without this rule, a workspace containing both "ana" and "anastasia" makes the
/// shorter name permanently unaddressable.
#[tokio::test]
async fn an_exact_name_beats_a_longer_substring_match() {
    let server = MockServer::start().await;
    mount_members(
        &server,
        serde_json::json!([
            { "user": { "uuid": "{1}", "display_name": "Ana", "nickname": "ana" } },
            { "user": { "uuid": "{2}", "display_name": "Anastasia", "nickname": "anastasia" } }
        ]),
    )
    .await;
    mount_default_reviewers(&server, serde_json::json!([])).await;

    let user = resolve_user(&client_for(&server.uri()), &slug(), "ANA", &[])
        .await
        .unwrap();
    assert_eq!(user.uuid.as_deref(), Some("{1}"));
}

#[tokio::test]
async fn no_match_errors_naming_the_query() {
    let server = MockServer::start().await;
    mount_members(&server, serde_json::json!([])).await;
    mount_default_reviewers(&server, serde_json::json!([])).await;

    let err = resolve_user(&client_for(&server.uri()), &slug(), "nobody", &[])
        .await
        .unwrap_err();
    match err {
        BbError::Config(message) => {
            assert!(message.contains("nobody"), "{message}");
            assert!(message.contains("uuid"), "{message}");
        }
        other => panic!("unexpected: {other:?}"),
    }
}

/// An email cannot become a uuid — bitbucket's member listings do not expose email
/// addresses — so it must fail at resolution with the normal message rather than
/// later, inside a write.
#[tokio::test]
async fn an_email_is_not_special_cased() {
    let server = MockServer::start().await;
    mount_members(&server, serde_json::json!([])).await;
    mount_default_reviewers(&server, serde_json::json!([])).await;

    let err = resolve_user(&client_for(&server.uri()), &slug(), "ana@example.com", &[])
        .await
        .unwrap_err();
    assert!(matches!(err, BbError::Config(_)), "got {err:?}");
}

/// The token may lack workspace scope. That must not make reviewer removal
/// impossible, because the smaller pools are enough for the common case.
#[tokio::test]
async fn a_403_on_members_falls_back_to_the_remaining_pool() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/workspaces/acme/members"))
        .respond_with(ResponseTemplate::new(403))
        .mount(&server)
        .await;
    mount_default_reviewers(
        &server,
        serde_json::json!([{ "uuid": "{p}", "display_name": "Patrick Stein" }]),
    )
    .await;

    let user = resolve_user(&client_for(&server.uri()), &slug(), "patrick", &[])
        .await
        .unwrap();
    assert_eq!(user.uuid.as_deref(), Some("{p}"));
}

/// `extra` is how `pr reviewers remove` can name someone who is tagged on the pull
/// request but is in neither the member list nor the default reviewers.
#[tokio::test]
async fn the_extra_pool_is_searched_too() {
    let server = MockServer::start().await;
    mount_members(&server, serde_json::json!([])).await;
    mount_default_reviewers(&server, serde_json::json!([])).await;

    let extra: Vec<bb_cli::api::models::User> =
        serde_json::from_value(serde_json::json!([{ "uuid": "{x}", "display_name": "Ex Ternal" }]))
            .unwrap();
    let user = resolve_user(&client_for(&server.uri()), &slug(), "ternal", &extra)
        .await
        .unwrap();
    assert_eq!(user.uuid.as_deref(), Some("{x}"));
}
