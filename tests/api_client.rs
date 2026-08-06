#![allow(clippy::unwrap_used)] // test code is exempt from the unwrap/expect ban

mod support;

use bb_cli::api::{repo_path, Page};
use bb_cli::error::BbError;
use bb_cli::repo::RepoSlug;
use serde::Deserialize;
use support::client_for;
use wiremock::matchers::{header, method, path, query_param};
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
