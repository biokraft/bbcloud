#![allow(clippy::unwrap_used)]

use assert_cmd::Command;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// The release payload shape `bb update` reads: only `tag_name` matters for
/// the comparison.
fn release_body(tag: &str) -> serde_json::Value {
    serde_json::json!({ "tag_name": tag, "assets": [] })
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

    let output = Command::cargo_bin("bb")
        .unwrap()
        .args(["update", "--json"])
        .env("BB_UPDATE_API_BASE", server.uri())
        .env("NO_COLOR", "1")
        .output()
        .unwrap();

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

    Command::cargo_bin("bb")
        .unwrap()
        .args(["update", "--json"])
        .env("BB_UPDATE_API_BASE", server.uri())
        .env("BB_EMAIL", "dev@example.com")
        .env("BB_TOKEN", "ATATT-super-secret-value")
        .env("NO_COLOR", "1")
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

    let output = Command::cargo_bin("bb")
        .unwrap()
        .args(["update", "--json"])
        .env("BB_UPDATE_API_BASE", server.uri())
        .env("NO_COLOR", "1")
        .output()
        .unwrap();

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

    let output = Command::cargo_bin("bb")
        .unwrap()
        .args(["update", "--json"])
        .env("BB_UPDATE_API_BASE", server.uri())
        .env("NO_COLOR", "1")
        .output()
        .unwrap();

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

/// A malformed tag must not be treated as an upgrade, and must not panic.
#[tokio::test]
async fn a_malformed_remote_tag_is_not_an_upgrade() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/biokraft/bbcloud/releases/latest"))
        .respond_with(ResponseTemplate::new(200).set_body_json(release_body("nightly")))
        .mount(&server)
        .await;

    let output = Command::cargo_bin("bb")
        .unwrap()
        .args(["update", "--json"])
        .env("BB_UPDATE_API_BASE", server.uri())
        .env("NO_COLOR", "1")
        .output()
        .unwrap();

    assert!(output.status.success(), "should exit 0, not panic");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(parsed["up_to_date"], serde_json::Value::Bool(true));
}
