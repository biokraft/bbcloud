#![allow(clippy::unwrap_used)] // test code is exempt from the unwrap/expect ban

mod support;

use bb_cli::api::models::{BuildState, BuildStatus};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

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
