#![allow(clippy::unwrap_used)]

//! The passive "a newer bb is available" notice that runs ahead of every
//! command. Driven through `bb skill status`, which needs no credentials and no
//! Bitbucket API, so what is asserted is the notice and nothing else.

use assert_cmd::Command;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn release_body(tag: &str) -> serde_json::Value {
    serde_json::json!({ "tag_name": tag, "assets": [] })
}

/// A tag comfortably ahead of whatever this build's version is, so the test
/// keeps meaning the same thing after every release bump.
fn newer_tag() -> String {
    let current = env!("CARGO_PKG_VERSION");
    let major: u64 = current.split('.').next().unwrap().parse().unwrap();
    format!("v{}.0.0", major + 1)
}

/// Every invocation points `HOME`/`XDG_CONFIG_HOME` at a fresh tempdir, so
/// the cache the check writes can never be the developer's real one, and each
/// test starts with no cache at all. `BB_NO_UPDATE_CHECK` is deliberately
/// *not* set here — this file is the one place that wants the check to run.
fn bb(api: &str) -> (Command, tempfile::TempDir) {
    let cfg = tempfile::tempdir().unwrap();
    let mut cmd = Command::cargo_bin("bb").unwrap();
    cmd.env("HOME", cfg.path())
        .env("XDG_CONFIG_HOME", cfg.path())
        .env("BB_UPDATE_API_BASE", api)
        .env("BB_KEYRING_DISABLE", "1")
        .env("BB_SKILL_NO_AUTO_REFRESH", "1")
        .env("NO_COLOR", "1");
    (cmd, cfg)
}

async fn releases(tag: &str, expect: u64) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/biokraft/bbcloud/releases/latest"))
        .respond_with(ResponseTemplate::new(200).set_body_json(release_body(tag)))
        .expect(expect)
        .mount(&server)
        .await;
    server
}

#[tokio::test]
async fn a_newer_release_is_announced_on_stderr() {
    let server = releases(&newer_tag(), 1).await;
    let (mut cmd, _cfg) = bb(&server.uri());
    let out = cmd.args(["skill", "status"]).output().unwrap();

    let stderr = String::from_utf8_lossy(&out.stderr);
    let expected = newer_tag().trim_start_matches('v').to_string();
    assert!(
        stderr.contains(&format!("bb {expected} is available")),
        "stderr must carry the notice, got: {stderr}"
    );
    assert!(
        stderr.contains("upgrade with:"),
        "the notice must name the upgrade command, got: {stderr}"
    );
    // The notice must never land on stdout: `--json` commands promise a bare
    // serde value there, and `bb skill status` promises its own table.
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains("is available"),
        "stdout must stay clean, got: {stdout}"
    );
}

#[tokio::test]
async fn json_mode_keeps_stdout_pure_and_still_notifies() {
    let server = releases(&newer_tag(), 1).await;
    let (mut cmd, _cfg) = bb(&server.uri());
    let out = cmd.args(["skill", "status", "--json"]).output().unwrap();

    let stdout = String::from_utf8_lossy(&out.stdout);
    serde_json::from_str::<serde_json::Value>(stdout.trim())
        .unwrap_or_else(|err| panic!("stdout must be a bare json value ({err}): {stdout}"));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("is available"),
        "an agent reads stderr, so --json must still be told, got: {stderr}"
    );
}

#[tokio::test]
async fn the_running_version_is_not_announced() {
    let server = releases(&format!("v{}", env!("CARGO_PKG_VERSION")), 1).await;
    let (mut cmd, _cfg) = bb(&server.uri());
    let out = cmd.args(["skill", "status"]).output().unwrap();

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("is available"),
        "an up-to-date install must stay silent, got: {stderr}"
    );
}

#[tokio::test]
async fn a_fresh_cache_makes_no_second_request() {
    // `expect(1)` is the assertion: the second invocation must be served
    // entirely from the cache the first one wrote.
    let server = releases(&newer_tag(), 1).await;

    let (mut first, cfg) = bb(&server.uri());
    first.args(["skill", "status"]).output().unwrap();

    let mut second = Command::cargo_bin("bb").unwrap();
    let out = second
        .env("HOME", cfg.path())
        .env("XDG_CONFIG_HOME", cfg.path())
        .env("BB_UPDATE_API_BASE", server.uri())
        .env("BB_KEYRING_DISABLE", "1")
        .env("BB_SKILL_NO_AUTO_REFRESH", "1")
        .env("NO_COLOR", "1")
        .args(["skill", "status"])
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("is available"),
        "the cached answer must still be announced, got: {stderr}"
    );
    assert!(
        cfg.path().join("bb").join("update-check.json").exists(),
        "the check must record its answer"
    );
    drop(server);
}

#[tokio::test]
async fn the_kill_switch_makes_no_request_and_prints_nothing() {
    let server = releases(&newer_tag(), 0).await;
    let (mut cmd, _cfg) = bb(&server.uri());
    let out = cmd
        .env("BB_NO_UPDATE_CHECK", "1")
        .args(["skill", "status"])
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("is available"),
        "BB_NO_UPDATE_CHECK must silence the notice, got: {stderr}"
    );
}

#[tokio::test]
async fn an_unreachable_release_api_is_silent() {
    // A server that is started and immediately dropped leaves a port nothing
    // is listening on, which is what being offline looks like from here.
    let uri = {
        let server = MockServer::start().await;
        server.uri()
    };
    let (mut cmd, _cfg) = bb(&uri);
    let out = cmd.args(["skill", "status"]).output().unwrap();

    assert!(out.status.success(), "the check must never fail a command");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("is available") && !stderr.contains("error"),
        "an unreachable release api must be swallowed, got: {stderr}"
    );
}

#[tokio::test]
async fn bb_update_does_not_also_run_the_passive_check() {
    // One request total: `bb update`'s own. A second would mean the passive
    // check ran ahead of it and duplicated both the request and the line.
    let server = releases(&format!("v{}", env!("CARGO_PKG_VERSION")), 1).await;
    let (mut cmd, _cfg) = bb(&server.uri());
    let out = cmd.args(["update", "--json"]).output().unwrap();

    assert!(out.status.success(), "update must succeed");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("is available"),
        "bb update reports the version itself, got: {stderr}"
    );
}
