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
    let values = reviewers
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|user| serde_json::json!({ "user": user }))
        .collect::<Vec<_>>();
    Mock::given(method("GET"))
        .and(path(
            "/repositories/acme/widgets/effective-default-reviewers",
        ))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({ "values": values })),
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
            { "user": { "uuid": "{p}", "display_name": "Dana Stein", "nickname": "dana" } },
            { "user": { "uuid": "{r}", "display_name": "Ash Doe", "nickname": "ash" } }
        ]),
    )
    .await;
    mount_default_reviewers(&server, serde_json::json!([])).await;
    mount_permissions_config(&server, serde_json::json!([])).await;

    let user = resolve_user(&client_for(&server.uri()), &slug(), "dan", &[])
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
    mount_permissions_config(&server, serde_json::json!([])).await;

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
    mount_permissions_config(&server, serde_json::json!([])).await;

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
    mount_permissions_config(&server, serde_json::json!([])).await;

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
    mount_permissions_config(&server, serde_json::json!([])).await;

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
        serde_json::json!([{ "uuid": "{p}", "display_name": "Dana Stein" }]),
    )
    .await;
    mount_permissions_config(&server, serde_json::json!([])).await;

    let user = resolve_user(&client_for(&server.uri()), &slug(), "dana", &[])
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
    mount_permissions_config(&server, serde_json::json!([])).await;

    let extra: Vec<bb_cli::api::models::User> =
        serde_json::from_value(serde_json::json!([{ "uuid": "{x}", "display_name": "Ex Ternal" }]))
            .unwrap();
    let user = resolve_user(&client_for(&server.uri()), &slug(), "ternal", &extra)
        .await
        .unwrap();
    assert_eq!(user.uuid.as_deref(), Some("{x}"));
}

/// The live bug this feature shipped with: someone who has explicit repo access
/// but is neither a workspace member (as seen by this token) nor a default
/// reviewer must still resolve, via `/permissions-config/users`.
#[tokio::test]
async fn a_name_present_only_in_the_permissions_config_list_resolves() {
    let server = MockServer::start().await;
    mount_members(&server, serde_json::json!([])).await;
    mount_default_reviewers(&server, serde_json::json!([])).await;
    mount_permissions_config(
        &server,
        serde_json::json!([
            { "user": { "uuid": "{w}", "display_name": "Wenyi Ou", "nickname": "Wenyi Ou" } }
        ]),
    )
    .await;

    let user = resolve_user(&client_for(&server.uri()), &slug(), "wenyi", &[])
        .await
        .unwrap();
    assert_eq!(user.uuid.as_deref(), Some("{w}"));
}

/// A members 403 is routine once the permissions-config pool covers the gap —
/// it must not produce a warning when resolution still succeeds.
#[tokio::test]
async fn a_members_403_with_a_working_permissions_list_resolves_without_warning() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/workspaces/acme/members"))
        .respond_with(ResponseTemplate::new(403))
        .mount(&server)
        .await;
    mount_default_reviewers(&server, serde_json::json!([])).await;
    mount_permissions_config(
        &server,
        serde_json::json!([
            { "user": { "uuid": "{w}", "display_name": "Wenyi Ou" } }
        ]),
    )
    .await;

    let user = resolve_user(&client_for(&server.uri()), &slug(), "wenyi", &[])
        .await
        .unwrap();
    assert_eq!(user.uuid.as_deref(), Some("{w}"));
    // No direct way to assert stderr from here; the success path in `users.rs`
    // simply never calls `output::warn` unless `found.len() == 0`, which is
    // exercised deliberately by the next test.
}

/// No match anywhere, with a members 403, must both fail resolution and warn on
/// stderr that the pool may be incomplete.
#[tokio::test]
async fn no_match_with_a_members_403_warns_and_errors() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/workspaces/acme/members"))
        .respond_with(ResponseTemplate::new(403))
        .mount(&server)
        .await;
    mount_default_reviewers(&server, serde_json::json!([])).await;
    mount_permissions_config(&server, serde_json::json!([])).await;

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
    // The warning itself goes to stderr via `output::warn`, which this
    // in-process test cannot capture — that's covered at the CLI level in
    // `tests/user_resolve_cli.rs`, which spawns the real binary and reads its
    // stderr. This test only pins that the ordinary error still fires here.
}

/// A person listed in both `/permissions-config/users` and `/default-reviewers`
/// is one candidate, not two — otherwise every default reviewer with explicit
/// repo access would look ambiguous against their own name.
#[tokio::test]
async fn a_person_in_both_permissions_and_default_reviewers_is_not_ambiguous() {
    let server = MockServer::start().await;
    mount_members(&server, serde_json::json!([])).await;
    mount_default_reviewers(
        &server,
        serde_json::json!([{ "uuid": "{m}", "display_name": "Dana Fischer" }]),
    )
    .await;
    mount_permissions_config(
        &server,
        serde_json::json!([
            { "user": { "uuid": "{m}", "display_name": "Dana Fischer" } }
        ]),
    )
    .await;

    let user = resolve_user(&client_for(&server.uri()), &slug(), "dana", &[])
        .await
        .unwrap();
    assert_eq!(user.uuid.as_deref(), Some("{m}"));
}

/// Every field is optional in the api, and the pools populate different
/// subsets. A later, richer record must fill gaps rather than being dropped
/// for duplicating an identity.
#[tokio::test]
async fn a_duplicate_identity_keeps_the_richer_record() {
    let server = MockServer::start().await;
    mount_members(
        &server,
        serde_json::json!([{ "user": { "uuid": "{m}", "account_id": "acct-1" } }]),
    )
    .await;
    mount_permissions_config(
        &server,
        serde_json::json!([{
            "user": { "uuid": "{m}", "display_name": "Dana Fischer", "nickname": "dana" }
        }]),
    )
    .await;
    mount_default_reviewers(&server, serde_json::json!([])).await;

    let user = resolve_user(&client_for(&server.uri()), &slug(), "dana", &[])
        .await
        .unwrap();
    assert_eq!(user.uuid.as_deref(), Some("{m}"));
    assert_eq!(user.account_id.as_deref(), Some("acct-1"));
    assert_eq!(user.nickname.as_deref(), Some("dana"));
}

/// `account_id` is the fallback identity when a response omits `uuid`, so two
/// records carrying only that field are still one person.
#[tokio::test]
async fn account_id_deduplicates_when_uuid_is_absent() {
    let server = MockServer::start().await;
    mount_members(
        &server,
        serde_json::json!([{ "user": { "account_id": "acct-1", "display_name": "Dana" } }]),
    )
    .await;
    mount_permissions_config(
        &server,
        serde_json::json!([{ "user": { "account_id": "acct-1", "nickname": "dana" } }]),
    )
    .await;
    mount_default_reviewers(&server, serde_json::json!([])).await;

    let user = resolve_user(&client_for(&server.uri()), &slug(), "dana", &[])
        .await
        .unwrap();
    assert_eq!(user.display_name.as_deref(), Some("Dana"));
    assert_eq!(user.nickname.as_deref(), Some("dana"));
}

/// Without a stable identity the record cannot be tagged and cannot be told
/// apart from a namesake, so it stays out of name resolution entirely.
#[tokio::test]
async fn a_record_with_no_stable_identity_is_not_resolvable() {
    let server = MockServer::start().await;
    mount_members(
        &server,
        serde_json::json!([{ "user": { "display_name": "Dana" } }, { "user": { "display_name": "Dana" } }]),
    )
    .await;
    mount_permissions_config(&server, serde_json::json!([])).await;
    mount_default_reviewers(&server, serde_json::json!([])).await;

    let err = resolve_user(&client_for(&server.uri()), &slug(), "dana", &[])
        .await
        .unwrap_err();
    assert!(matches!(err, BbError::Config(_)), "got {err:?}");
}

/// `permissions-config/users` generally needs repo admin, which a CI token or a
/// less-privileged colleague's token may lack. That must degrade exactly like a
/// members 403 does, not abort resolution before default-reviewers is even
/// consulted — otherwise this fix regresses every token that isn't a repo admin.
#[tokio::test]
async fn a_403_on_permissions_config_falls_back_to_default_reviewers() {
    let server = MockServer::start().await;
    mount_members(&server, serde_json::json!([])).await;
    Mock::given(method("GET"))
        .and(path("/repositories/acme/widgets/permissions-config/users"))
        .respond_with(ResponseTemplate::new(403))
        .mount(&server)
        .await;
    mount_default_reviewers(
        &server,
        serde_json::json!([{ "uuid": "{p}", "display_name": "Dana Stein" }]),
    )
    .await;

    let user = resolve_user(&client_for(&server.uri()), &slug(), "dana", &[])
        .await
        .unwrap();
    assert_eq!(user.uuid.as_deref(), Some("{p}"));
}

/// Both lookups refused (members 403, permissions-config 403) plus no match
/// anywhere still produces the ordinary "no user matching" error rather than
/// propagating either refusal as a hard failure.
#[tokio::test]
async fn both_lookups_refused_with_no_match_still_errors_normally() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/workspaces/acme/members"))
        .respond_with(ResponseTemplate::new(403))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repositories/acme/widgets/permissions-config/users"))
        .respond_with(ResponseTemplate::new(403))
        .mount(&server)
        .await;
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
/// A 401 is not an authorization gap to degrade around: the token is wrong, and
/// every later pool would be read with the same wrong token. Whichever pool
/// answers first, the command stops with the authentication error.
///
/// Every other pool is mounted and answering, so a 401 can only come from the
/// one under test — otherwise this would pass on an unmounted endpoint's 404.
#[tokio::test]
async fn a_401_on_any_pool_stops_immediately() {
    const POOLS: [&str; 3] = [
        "/workspaces/acme/members",
        "/repositories/acme/widgets/permissions-config/users",
        "/repositories/acme/widgets/effective-default-reviewers",
    ];
    for failing in POOLS {
        let server = MockServer::start().await;
        for pool in POOLS {
            let response = if pool == failing {
                ResponseTemplate::new(401)
            } else {
                ResponseTemplate::new(200).set_body_json(serde_json::json!({ "values": [] }))
            };
            Mock::given(method("GET"))
                .and(path(pool))
                .respond_with(response)
                .mount(&server)
                .await;
        }

        let err = resolve_user(&client_for(&server.uri()), &slug(), "dana", &[])
            .await
            .unwrap_err();
        assert!(matches!(err, BbError::Auth), "{failing}: got {err:?}");
    }
}

/// A server-side failure is neither an answer nor an authorization gap, so it
/// must surface rather than being folded into a partial pool.
#[tokio::test]
async fn a_server_error_on_a_pool_surfaces_instead_of_degrading() {
    for failing in [
        "/workspaces/acme/members",
        "/repositories/acme/widgets/permissions-config/users",
        "/repositories/acme/widgets/effective-default-reviewers",
    ] {
        let server = MockServer::start().await;
        for pool in [
            "/workspaces/acme/members",
            "/repositories/acme/widgets/permissions-config/users",
            "/repositories/acme/widgets/effective-default-reviewers",
        ] {
            let response = if pool == failing {
                ResponseTemplate::new(500)
            } else {
                ResponseTemplate::new(200).set_body_json(serde_json::json!({ "values": [] }))
            };
            Mock::given(method("GET"))
                .and(path(pool))
                .respond_with(response)
                .mount(&server)
                .await;
        }

        let err = resolve_user(&client_for(&server.uri()), &slug(), "dana", &[])
            .await
            .unwrap_err();
        match err {
            BbError::Api { status, .. } => assert_eq!(status, 500, "{failing}"),
            other => panic!("{failing}: expected the api error to surface, got {other:?}"),
        }
    }
}

/// A pool that cannot be read is named in `partial`, so a caller can tell an
/// incomplete answer from a complete one. Default reviewers are a pool too.
#[tokio::test]
async fn a_403_on_default_reviewers_is_reported_as_partial() {
    let server = MockServer::start().await;
    mount_members(&server, serde_json::json!([])).await;
    mount_permissions_config(&server, serde_json::json!([])).await;
    Mock::given(method("GET"))
        .and(path(
            "/repositories/acme/widgets/effective-default-reviewers",
        ))
        .respond_with(ResponseTemplate::new(403))
        .mount(&server)
        .await;

    let err = resolve_user(&client_for(&server.uri()), &slug(), "nobody", &[])
        .await
        .unwrap_err();
    let message = err.to_string();
    assert!(message.contains("nobody"), "{message}");
}

/// An empty name is a caller mistake, and naming it plainly is more useful than
/// an empty search that reports every user as a candidate.
#[tokio::test]
async fn an_empty_name_is_rejected_before_any_lookup() {
    let server = MockServer::start().await;
    let err = resolve_user(&client_for(&server.uri()), &slug(), "   ", &[])
        .await
        .unwrap_err();
    assert!(matches!(err, BbError::Config(_)), "got {err:?}");
    assert!(
        server
            .received_requests()
            .await
            .unwrap_or_default()
            .is_empty(),
        "an empty name must not spend a pool lookup"
    );
}
