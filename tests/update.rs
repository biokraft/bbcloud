#![allow(clippy::unwrap_used)]

use assert_cmd::Command;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// The release payload shape `bb update` reads: only `tag_name` matters for
/// the comparison.
fn release_body(tag: &str) -> serde_json::Value {
    serde_json::json!({ "tag_name": tag, "assets": [] })
}

/// Every binary invocation in this file must go through here: it points
/// `HOME` and `XDG_CONFIG_HOME` at a per-test tempdir (so `refresh_tracked`'s
/// unconditional `save_state` can never touch the developer's real
/// `~/.config/bb/skills.json`) and disables the keyring, since `bb update`
/// should never reach it. Returns the tempdir too so callers that need to
/// assert on the config path can keep it alive.
fn bb(api: &str) -> (Command, tempfile::TempDir) {
    let cfg = tempfile::tempdir().unwrap();
    let mut cmd = Command::cargo_bin("bb").unwrap();
    cmd.env("HOME", cfg.path())
        .env("XDG_CONFIG_HOME", cfg.path())
        .env("BB_UPDATE_API_BASE", api)
        .env("BB_KEYRING_DISABLE", "1")
        .env("NO_COLOR", "1");
    (cmd, cfg)
}

#[tokio::test]
async fn reports_up_to_date_in_json_when_the_latest_tag_matches() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/biokraft/bbcloud/releases/latest"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(release_body(&format!("v{}", env!("CARGO_PKG_VERSION")))),
        )
        .mount(&server)
        .await;

    let (mut cmd, _cfg) = bb(&server.uri());
    let output = cmd.args(["update", "--json"]).output().unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("stdout is not pure json: {e}\n{stdout}"));
    assert_eq!(parsed["up_to_date"], serde_json::Value::Bool(true));
    assert_eq!(parsed["current"], env!("CARGO_PKG_VERSION"));
    assert_eq!(parsed["latest"], format!("v{}", env!("CARGO_PKG_VERSION")));
}

/// The most important test in this file. `api::Client` attaches the Basic auth
/// header unconditionally; `update` must NOT use it, because the token belongs
/// to Bitbucket and this request goes to GitHub.
#[tokio::test]
async fn the_api_token_is_never_sent_to_the_release_host() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/biokraft/bbcloud/releases/latest"))
        .respond_with(ResponseTemplate::new(200).set_body_json(release_body("v0.0.1")))
        .mount(&server)
        .await;

    let (mut cmd, _cfg) = bb(&server.uri());
    cmd.args(["update", "--json"])
        .env("BB_EMAIL", "dev@example.com")
        .env("BB_TOKEN", "ATATT-super-secret-value")
        .output()
        .unwrap();

    for request in server.received_requests().await.unwrap() {
        assert!(
            request.headers.get("authorization").is_none(),
            "update sent an Authorization header to the release host"
        );
        let serialized = format!("{:?}", request.headers);
        assert!(
            !serialized.contains("ATATT-super-secret-value"),
            "the api token leaked into a request header: {serialized}"
        );
    }
}

/// A newer release whose assets are missing must fail loudly and leave the
/// running binary byte-for-byte unchanged. This is the verify-before-write
/// guarantee: nothing is written next to the executable until a download has
/// been fetched AND its digest matched.
#[tokio::test]
async fn a_newer_release_with_missing_assets_fails_without_touching_the_binary() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/biokraft/bbcloud/releases/latest"))
        .respond_with(ResponseTemplate::new(200).set_body_json(release_body("v99.0.0")))
        .mount(&server)
        .await;

    let exe = assert_cmd::cargo::cargo_bin("bb");
    let before = std::fs::read(&exe).unwrap();

    let (mut cmd, _cfg) = bb(&server.uri());
    let output = cmd.args(["update", "--json"]).output().unwrap();

    assert!(!output.status.success(), "a failed update must not exit 0");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("missing") || stderr.contains("asset"),
        "the error should name the missing asset, got: {stderr}"
    );
    assert_eq!(
        std::fs::read(&exe).unwrap(),
        before,
        "the running binary was modified despite the update failing"
    );
}

/// Builds a valid tar.gz whose only entry named `bb` is a link of the given
/// type pointing at `target`. The header's size must be set explicitly to 0
/// and the entry type set explicitly — `tar::Header::new_gnu()` otherwise
/// leaves the size field blank, which makes the *reader* fail during tar
/// parsing (`numeric field was not a number: ... for bb`) before
/// `entry_type()` is ever consulted. An archive that fails to parse would
/// make this test pass for the wrong reason: it must be well-formed so the
/// rejection comes from the `is_file()` check under test, not from a parse
/// error that the pre-fix code would have hit identically.
fn build_link_archive(entry_type: tar::EntryType, target: &str) -> Vec<u8> {
    use std::io::Write;

    let mut tar_bytes = Vec::new();
    {
        let mut builder = tar::Builder::new(&mut tar_bytes);
        let mut header = tar::Header::new_gnu();
        header.set_size(0);
        header.set_entry_type(entry_type);
        header.set_mode(0o644);
        builder.append_link(&mut header, "bb", target).unwrap();
        builder.finish().unwrap();
    }
    let mut archive_bytes = Vec::new();
    {
        let mut encoder =
            flate2::write::GzEncoder::new(&mut archive_bytes, flate2::Compression::default());
        encoder.write_all(&tar_bytes).unwrap();
        encoder.finish().unwrap();
    }
    archive_bytes
}

/// Runs `bb update` against a `bb`-named archive entry of the given link
/// type pointing at a freshly created victim file, and asserts the whole
/// verify-before-write / reject-non-file contract holds: non-zero exit, no
/// staged file left behind, the victim untouched, and the running binary
/// byte-for-byte unchanged.
async fn assert_link_entry_is_rejected(entry_type: tar::EntryType) {
    use sha2::{Digest, Sha256};
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    let tmp = tempfile::tempdir().unwrap();
    let victim = tmp.path().join("victim");
    std::fs::write(&victim, b"do not touch me").unwrap();
    #[cfg(unix)]
    std::fs::set_permissions(&victim, std::fs::Permissions::from_mode(0o644)).unwrap();

    let archive_bytes = build_link_archive(entry_type, victim.to_str().unwrap());
    let digest = format!("{:x}", Sha256::digest(&archive_bytes));

    let triple = bb_cli::commands::update::current_triple().unwrap();
    let tag = "v99.0.0";
    let (archive_name, checksum_name) = bb_cli::commands::update::asset_names(tag, triple);

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/biokraft/bbcloud/releases/latest"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "tag_name": tag,
            "assets": [
                {
                    "name": archive_name,
                    "browser_download_url": format!("{}/assets/{archive_name}", server.uri()),
                },
                {
                    "name": checksum_name,
                    "browser_download_url": format!("{}/assets/{checksum_name}", server.uri()),
                },
            ],
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/assets/{archive_name}")))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(archive_bytes))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/assets/{checksum_name}")))
        .respond_with(ResponseTemplate::new(200).set_body_string(digest))
        .mount(&server)
        .await;

    let exe = assert_cmd::cargo::cargo_bin("bb");
    let before = std::fs::read(&exe).unwrap();
    let exe_dir = exe.parent().unwrap().to_path_buf();

    let (mut cmd, _cfg) = bb(&server.uri());
    let output = cmd.args(["update", "--json"]).output().unwrap();

    assert!(
        !output.status.success(),
        "a rejected {entry_type:?} entry must not exit 0"
    );

    for entry in std::fs::read_dir(&exe_dir).unwrap() {
        let name = entry.unwrap().file_name();
        assert!(
            !name.to_string_lossy().starts_with(".bb-update-staged"),
            "a staged file was left behind: {name:?}"
        );
    }

    let victim_meta = std::fs::symlink_metadata(&victim).unwrap();
    assert!(
        !victim_meta.file_type().is_symlink(),
        "victim should still be a regular file"
    );
    #[cfg(unix)]
    assert_eq!(
        victim_meta.permissions().mode() & 0o777,
        0o644,
        "victim's permissions must be untouched"
    );
    assert_eq!(
        std::fs::read(&victim).unwrap(),
        b"do not touch me",
        "victim's contents must be untouched"
    );

    assert_eq!(
        std::fs::read(&exe).unwrap(),
        before,
        "the running binary was modified despite the {entry_type:?} rejection"
    );
}

/// Pins the Critical fix: a `bb` entry that is a symlink rather than a
/// regular file must be rejected, not unpacked. `tar::Entry::unpack` skips
/// link validation when given an explicit destination with no `target_base`,
/// so unpacking a symlink entry directly would chmod/replace whatever it
/// points at, entirely outside the install directory.
#[tokio::test]
async fn a_symlink_bb_entry_is_rejected_and_leaves_everything_untouched() {
    assert_link_entry_is_rejected(tar::EntryType::Symlink).await;
}

/// Same contract, for a hard-link entry. `Entry::unpack`'s link-handling
/// branch covers both link types, and the original finding named both.
#[tokio::test]
async fn a_hard_link_bb_entry_is_rejected_and_leaves_everything_untouched() {
    assert_link_entry_is_rejected(tar::EntryType::Link).await;
}

/// 403 with `x-ratelimit-remaining: 0` and a valid reset header must be
/// reported as a rate limit, with the retry time derived from the header
/// (not hardcoded, so the assertion holds in any timezone).
#[tokio::test]
async fn rate_limited_403_with_reset_header_reports_retry_time() {
    let server = MockServer::start().await;
    let reset_epoch: i64 = 1_786_452_151;
    Mock::given(method("GET"))
        .and(path("/repos/biokraft/bbcloud/releases/latest"))
        .respond_with(
            ResponseTemplate::new(403)
                .insert_header("x-ratelimit-remaining", "0")
                .insert_header("x-ratelimit-reset", reset_epoch.to_string().as_str()),
        )
        .mount(&server)
        .await;

    let (mut cmd, _cfg) = bb(&server.uri());
    let output = cmd.args(["update", "--json"]).output().unwrap();

    assert!(
        !output.status.success(),
        "a rate-limited update must not exit 0"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("rate limit"),
        "stderr should name the rate limit, got: {stderr}"
    );

    let expected_time = chrono::DateTime::from_timestamp(reset_epoch, 0)
        .unwrap()
        .with_timezone(&chrono::Local)
        .format("%H:%M")
        .to_string();
    assert!(
        stderr.contains(&expected_time),
        "stderr should contain the retry time {expected_time}, got: {stderr}"
    );
}

/// A missing or unparseable reset header must never yield a bogus 1970
/// timestamp or a panic — the retry time is simply omitted.
#[tokio::test]
async fn rate_limited_403_without_reset_header_omits_the_time_without_panicking() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/biokraft/bbcloud/releases/latest"))
        .respond_with(ResponseTemplate::new(403).insert_header("x-ratelimit-remaining", "0"))
        .mount(&server)
        .await;

    let (mut cmd, _cfg) = bb(&server.uri());
    let output = cmd.args(["update", "--json"]).output().unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("rate limit"),
        "stderr should still name the rate limit, got: {stderr}"
    );
    assert!(
        !stderr.contains("1970"),
        "stderr must never show a 1970 fallback timestamp, got: {stderr}"
    );
}

/// A 403 that carries no rate-limit signal (or a non-zero remaining count)
/// must be reported as a plain release-api error, and must not falsely
/// claim a rate limit.
#[tokio::test]
async fn non_rate_limit_403_reports_a_plain_release_api_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/biokraft/bbcloud/releases/latest"))
        .respond_with(ResponseTemplate::new(403))
        .mount(&server)
        .await;

    let (mut cmd, _cfg) = bb(&server.uri());
    let output = cmd.args(["update", "--json"]).output().unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("release api error 403"),
        "stderr should name the release api error, got: {stderr}"
    );
    assert!(
        !stderr.contains("rate limit"),
        "a plain 403 must not falsely claim a rate limit, got: {stderr}"
    );
}

/// A 500 must be reported honestly as a release-api error, and must never
/// blame Bitbucket — the request went to GitHub.
#[tokio::test]
async fn server_error_500_reports_release_api_error_without_blaming_bitbucket() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/biokraft/bbcloud/releases/latest"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let (mut cmd, _cfg) = bb(&server.uri());
    let output = cmd.args(["update", "--json"]).output().unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("release api error 500"),
        "stderr should say release api error 500, got: {stderr}"
    );
    assert!(
        !stderr.to_lowercase().contains("bitbucket"),
        "stderr must never blame bitbucket for a release-api failure, got: {stderr}"
    );
}

/// A malformed tag must not be treated as an upgrade, and must not panic.
#[tokio::test]
async fn a_malformed_remote_tag_is_not_an_upgrade() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/biokraft/bbcloud/releases/latest"))
        .respond_with(ResponseTemplate::new(200).set_body_json(release_body("nightly")))
        .mount(&server)
        .await;

    let (mut cmd, _cfg) = bb(&server.uri());
    let output = cmd.args(["update", "--json"]).output().unwrap();

    assert!(output.status.success(), "should exit 0, not panic");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(parsed["up_to_date"], serde_json::Value::Bool(true));
}

/// Regression for the bug this file used to have: every spawned `bb`
/// invocation resolved the developer's real `~/.config/bb/skills.json`
/// because none of them overrode `HOME`/`XDG_CONFIG_HOME`, and
/// `refresh_tracked` (which `update` calls on every path, including
/// up-to-date) ends in an unconditional `save_state`. Simulates "the
/// developer's real config" as a second tempdir that is never passed to the
/// child process at all — only `bb()`'s overridden `cfg` is — and proves the
/// write landed only inside the override, never inside the stand-in for the
/// real one.
#[tokio::test]
async fn update_never_touches_the_real_config_path() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/biokraft/bbcloud/releases/latest"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(release_body(&format!("v{}", env!("CARGO_PKG_VERSION")))),
        )
        .mount(&server)
        .await;
    let stand_in_for_real_home = tempfile::tempdir().unwrap();
    let stand_in_real_state = stand_in_for_real_home
        .path()
        .join(".config")
        .join("bb")
        .join("skills.json");

    let (mut cmd, cfg) = bb(&server.uri());
    cmd.arg("update").assert().success();

    assert!(
        !stand_in_real_state.exists(),
        "bb update must never write outside the HOME/XDG_CONFIG_HOME override: {} was created",
        stand_in_real_state.display()
    );
    // Sanity: the override itself was actually exercised (refresh_tracked's
    // unconditional save_state writes here even with nothing tracked), so
    // the assertion above isn't just "nothing ran at all".
    assert!(
        cfg.path().join("bb").join("skills.json").exists(),
        "the overridden config dir should be the one bb actually used"
    );
}
