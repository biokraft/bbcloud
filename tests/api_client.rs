#![allow(clippy::unwrap_used)] // test code is exempt from the unwrap/expect ban

mod support;

use bb_cli::api::{repo_path, Page};
use bb_cli::error::BbError;
use bb_cli::repo::RepoSlug;
use serde::Deserialize;
use support::client_for;
use wiremock::matchers::{body_json, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[derive(Debug, Deserialize)]
struct Item {
    id: u64,
}

#[tokio::test]
async fn sends_basic_auth_header() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/user"))
        // base64("dev@example.com:t0ken-value")
        .and(header(
            "authorization",
            "Basic ZGV2QGV4YW1wbGUuY29tOnQwa2VuLXZhbHVl",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"id": 1})))
        .mount(&server)
        .await;

    let item: Item = client_for(&server.uri()).get_json("/user").await.unwrap();
    assert_eq!(item.id, 1);
}

#[tokio::test]
async fn does_not_follow_redirects() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/user"))
        .respond_with(
            ResponseTemplate::new(302).insert_header("location", "https://evil.example.com/steal"),
        )
        .mount(&server)
        .await;

    let err = client_for(&server.uri())
        .get_json::<Item>("/user")
        .await
        .unwrap_err();
    // A 3xx is surfaced as an api error rather than transparently followed.
    assert!(
        matches!(err, BbError::Api { status: 302, .. }),
        "got {err:?}"
    );
}

#[tokio::test]
async fn maps_401_to_auth_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/user"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;

    let err = client_for(&server.uri())
        .get_json::<Item>("/user")
        .await
        .unwrap_err();
    assert!(matches!(err, BbError::Auth));
}

#[tokio::test]
async fn maps_404_to_not_found() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/nope"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let err = client_for(&server.uri())
        .get_json::<Item>("/nope")
        .await
        .unwrap_err();
    assert!(matches!(err, BbError::NotFound));
}

#[tokio::test]
async fn error_message_uses_api_error_field_not_raw_body() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/boom"))
        .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
            "type": "error",
            "error": { "message": "branch not found" }
        })))
        .mount(&server)
        .await;

    let err = client_for(&server.uri())
        .get_json::<Item>("/boom")
        .await
        .unwrap_err();
    match err {
        BbError::Api { status, message } => {
            assert_eq!(status, 400);
            assert_eq!(message, "branch not found");
        }
        other => panic!("unexpected: {other:?}"),
    }
}

#[tokio::test]
async fn paginate_follows_next_links() {
    let server = MockServer::start().await;
    let page_two = format!("{}/things?page=2", server.uri());

    Mock::given(method("GET"))
        .and(path("/things"))
        .and(query_param("page", "2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "values": [{"id": 3}]
        })))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/things"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "values": [{"id": 1}, {"id": 2}],
            "next": page_two
        })))
        .mount(&server)
        .await;

    let items: Vec<Item> = client_for(&server.uri()).paginate("/things").await.unwrap();
    let ids: Vec<u64> = items.iter().map(|i| i.id).collect();
    assert_eq!(ids, vec![1, 2, 3]);
}

#[tokio::test]
async fn paginate_stops_when_next_repeats_the_same_url() {
    let server = MockServer::start().await;
    let self_link = format!("{}/loop", server.uri());

    // Every response points `next` back at this same url.
    Mock::given(method("GET"))
        .and(path("/loop"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "values": [{ "id": 1 }],
            "next": self_link
        })))
        .mount(&server)
        .await;

    let items: Vec<Item> = client_for(&server.uri()).paginate("/loop").await.unwrap();
    // Without a guard this collects 100 copies (the page cap). With one, it stops
    // as soon as the link repeats.
    assert_eq!(
        items.len(),
        1,
        "expected the repeating link to stop pagination"
    );
}

#[tokio::test]
async fn get_text_returns_raw_diff() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/diff"))
        .respond_with(ResponseTemplate::new(200).set_body_string("--- a\n+++ b\n"))
        .mount(&server)
        .await;

    let text = client_for(&server.uri()).get_text("/diff").await.unwrap();
    assert!(text.starts_with("--- a"));
}

#[test]
fn repo_path_prefixes_repositories() {
    let slug = RepoSlug::parse("acme/widgets").unwrap();
    assert_eq!(
        repo_path(&slug, "/pullrequests"),
        "/repositories/acme/widgets/pullrequests"
    );
}

#[test]
fn page_defaults_are_forgiving() {
    let page: Page<Item> = serde_json::from_str("{}").unwrap();
    assert!(page.values.is_empty());
    assert!(page.next.is_none());
}

#[tokio::test]
async fn maps_403_to_a_scope_hint() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/forbidden"))
        .respond_with(ResponseTemplate::new(403))
        .mount(&server)
        .await;

    let err = client_for(&server.uri())
        .get_json::<Item>("/forbidden")
        .await
        .unwrap_err();
    match err {
        BbError::Api { status, message } => {
            assert_eq!(status, 403);
            // The message must point at the likely cause rather than echo the body.
            assert!(message.contains("scope"), "unhelpful message: {message}");
        }
        other => panic!("unexpected: {other:?}"),
    }
}

#[tokio::test]
async fn maps_403_with_api_message_to_include_it() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/forbidden"))
        .respond_with(ResponseTemplate::new(403).set_body_json(serde_json::json!({
            "type": "error",
            "error": {"message": "Access denied. You must be granted read:project:bitbucket scope."}
        })))
        .mount(&server)
        .await;

    let err = client_for(&server.uri())
        .get_json::<Item>("/forbidden")
        .await
        .unwrap_err();
    match err {
        BbError::Api { status, message } => {
            assert_eq!(status, 403);
            assert!(
                message
                    .contains("Access denied. You must be granted read:project:bitbucket scope."),
                "missing api message: {message}"
            );
            assert!(message.contains("scope"), "missing scope hint: {message}");
            // Nothing else from the body leaks through.
            assert!(!message.contains("\"type\""), "leaked raw body: {message}");
        }
        other => panic!("unexpected: {other:?}"),
    }
}

#[tokio::test]
async fn maps_403_with_no_usable_message_to_the_fixed_wording() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/forbidden"))
        .respond_with(ResponseTemplate::new(403).set_body_string("not json"))
        .mount(&server)
        .await;

    let err = client_for(&server.uri())
        .get_json::<Item>("/forbidden")
        .await
        .unwrap_err();
    match err {
        BbError::Api { status, message } => {
            assert_eq!(status, 403);
            assert!(
                message.contains("the token may lack the required scope"),
                "unhelpful message: {message}"
            );
            assert!(!message.contains("not json"), "leaked raw body: {message}");
        }
        other => panic!("unexpected: {other:?}"),
    }
}

#[tokio::test]
async fn maps_429_to_a_rate_limit_message() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/limited"))
        .respond_with(ResponseTemplate::new(429))
        .mount(&server)
        .await;

    let err = client_for(&server.uri())
        .get_json::<Item>("/limited")
        .await
        .unwrap_err();
    match err {
        BbError::Api { status, message } => {
            assert_eq!(status, 429);
            assert!(
                message.contains("rate limited"),
                "unhelpful message: {message}"
            );
        }
        other => panic!("unexpected: {other:?}"),
    }
}

#[tokio::test]
async fn put_json_sends_the_body_and_parses_the_response() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/thing/1"))
        .and(body_json(serde_json::json!({"content": "edited"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"id": 42})))
        .mount(&server)
        .await;

    let item: Item = client_for(&server.uri())
        .put_json("/thing/1", &serde_json::json!({"content": "edited"}))
        .await
        .unwrap();
    assert_eq!(item.id, 42);
}

/// A server that keeps handing out fresh `next` links must not be followed forever.
#[tokio::test]
async fn paginate_stops_at_the_page_cap() {
    let server = MockServer::start().await;
    let base = server.uri();

    // Page N links to page N+1, each a distinct url so the repeat-detection guard
    // does not fire — only the hard page cap can stop this.
    for n in 0..150u32 {
        let next = format!("{base}/pages?page={}", n + 1);
        Mock::given(method("GET"))
            .and(path("/pages"))
            .and(query_param("page", n.to_string()))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "values": [{"id": n}],
                "next": next
            })))
            .mount(&server)
            .await;
    }

    let items: Vec<Item> = client_for(&server.uri())
        .paginate("/pages?page=0")
        .await
        .unwrap();
    // MAX_PAGES is 100 in src/api/mod.rs.
    assert_eq!(items.len(), 100, "expected the page cap to stop pagination");
}

// Bitbucket's `/pullrequests/{id}/diff` endpoint answers 302 with a Location on
// the same origin. The client must follow that, still carrying the credentials,
// or `bb pr diff` fails with "bitbucket api error 302: Found".
#[tokio::test]
async fn follows_a_same_origin_redirect_and_replays_auth() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/pullrequests/1/diff"))
        .respond_with(ResponseTemplate::new(302).insert_header(
            "location",
            format!("{}/diff/abc..def", server.uri()).as_str(),
        ))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/diff/abc..def"))
        .and(header(
            "authorization",
            "Basic ZGV2QGV4YW1wbGUuY29tOnQwa2VuLXZhbHVl",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_string("diff --git a/x b/x\n"))
        .mount(&server)
        .await;

    let text = client_for(&server.uri())
        .get_text("/pullrequests/1/diff")
        .await
        .unwrap();
    assert_eq!(text, "diff --git a/x b/x\n");
}

// The credentials must never reach another origin, so a cross-origin redirect is
// not followed and the 302 surfaces as an error instead.
#[tokio::test]
async fn does_not_follow_a_cross_origin_redirect() {
    let elsewhere = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_string("secret"))
        .mount(&elsewhere)
        .await;

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/pullrequests/1/diff"))
        .respond_with(
            ResponseTemplate::new(302)
                .insert_header("location", format!("{}/steal", elsewhere.uri()).as_str()),
        )
        .mount(&server)
        .await;

    let err = client_for(&server.uri())
        .get_text("/pullrequests/1/diff")
        .await
        .unwrap_err();
    assert!(
        matches!(err, BbError::Api { status: 302, .. }),
        "expected the cross-origin redirect to stop, got {err:?}"
    );
    assert!(
        elsewhere
            .received_requests()
            .await
            .unwrap_or_default()
            .is_empty(),
        "the client sent the credentials to another origin"
    );
}

// A redirect that never terminates must stop at the hop cap rather than loop.
#[tokio::test]
async fn stops_a_redirect_loop_at_the_hop_cap() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/loop"))
        .respond_with(
            ResponseTemplate::new(302)
                .insert_header("location", format!("{}/loop", server.uri()).as_str()),
        )
        .mount(&server)
        .await;

    let err = client_for(&server.uri())
        .get_text("/loop")
        .await
        .unwrap_err();
    assert!(
        matches!(err, BbError::Api { status: 302, .. }),
        "expected the hop cap to stop the loop, got {err:?}"
    );
    // MAX_REDIRECTS is 5 in src/api/mod.rs, so the server sees the original
    // request plus five followed redirects and nothing more.
    let hops = server.received_requests().await.unwrap_or_default().len();
    assert_eq!(hops, 6, "expected exactly 5 followed redirects");
}
