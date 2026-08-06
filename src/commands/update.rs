use crate::error::{BbError, Result};
use crate::output::{self, Format};
use serde::Deserialize;
use std::path::Path;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallKind {
    Homebrew,
    Cargo,
    Standalone,
}

/// Decides who owns the binary at `exe` from its path alone. Overwriting a
/// package-manager-owned file would leave brew or cargo believing it manages
/// a file it no longer controls, so those cases delegate instead.
pub fn classify_install(exe: &Path) -> InstallKind {
    let path = exe.to_string_lossy();
    if path.contains("/homebrew/") || path.contains("/Cellar/") || path.contains("/linuxbrew/") {
        return InstallKind::Homebrew;
    }
    if path.contains("/.cargo/bin/") {
        return InstallKind::Cargo;
    }
    InstallKind::Standalone
}

/// Parses `1.2.3` or `v1.2.3`. Returns `None` for anything else, including
/// four-component versions and pre-release suffixes.
pub fn parse_version(text: &str) -> Option<(u64, u64, u64)> {
    let trimmed = text.trim().trim_start_matches('v');
    let mut parts = trimmed.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

/// True only when both versions parse and `latest` is strictly greater. An
/// unparseable remote tag is never an upgrade, so a malformed API response
/// cannot trigger a download.
pub fn is_newer(latest: &str, current: &str) -> bool {
    match (parse_version(latest), parse_version(current)) {
        (Some(l), Some(c)) => l > c,
        _ => false,
    }
}

/// The target triple of the running binary, or `None` on a platform this
/// project does not publish binaries for.
pub fn current_triple() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Some("aarch64-apple-darwin"),
        ("macos", "x86_64") => Some("x86_64-apple-darwin"),
        ("linux", "x86_64") => Some("x86_64-unknown-linux-gnu"),
        ("linux", "aarch64") => Some("aarch64-unknown-linux-gnu"),
        _ => None,
    }
}

/// Archive and checksum asset names for a tag and triple. Must match the
/// `archive:` pattern in `.github/workflows/release.yml`, which is
/// `bbcloud-$tag-$target`; cargo-binstall's default templates key off the
/// crate name, which is why the prefix is `bbcloud` and not the binary name.
pub fn asset_names(tag: &str, triple: &str) -> (String, String) {
    let archive = format!("bbcloud-{tag}-{triple}.tar.gz");
    let checksum = format!("{archive}.sha256");
    (archive, checksum)
}

pub const DEFAULT_RELEASE_API: &str = "https://api.github.com";

#[derive(Debug, Deserialize)]
struct ReleaseAsset {
    name: String,
    browser_download_url: String,
}

#[derive(Debug, Deserialize)]
struct Release {
    tag_name: String,
    #[serde(default)]
    assets: Vec<ReleaseAsset>,
}

/// A client with no credentials attached. The Bitbucket `api::Client` always
/// sends the Basic auth header, and that token must never reach another host.
///
/// This client deliberately diverges from two of `api::Client`'s rules, and
/// both divergences are safe only because no credential is ever attached
/// here: redirects are followed (default policy) because GitHub asset
/// download URLs redirect to `objects.githubusercontent.com`, and the total
/// timeout is longer than the API client's because this transfers a compiled
/// binary rather than a JSON page.
fn release_client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(120))
        .user_agent(concat!("bbcloud/", env!("CARGO_PKG_VERSION")))
        .build()?)
}

/// Archive downloads are capped generously for this binary but not
/// unbounded, so a checksum-valid bomb can't exhaust memory or disk. Enforced
/// while the body streams in (`fetch_bounded`), not after it is fully
/// buffered, since `Content-Length` is attacker-controlled and may be absent.
const MAX_ARCHIVE_BYTES: u64 = 64 * 1024 * 1024;
/// Decompressed output is capped separately: gzip can amplify a small
/// archive into a much larger stream.
const MAX_UNPACKED_BYTES: u64 = 200 * 1024 * 1024;
/// The `.sha256` body is a hex digest and a filename; a few KB is generous
/// and keeps a hostile checksum-file response from being buffered unbounded.
const MAX_CHECKSUM_BYTES: u64 = 4 * 1024;
/// The release metadata JSON (tag name plus asset list) is a few KB in
/// practice; 1 MiB is generous headroom while still keeping a hostile or
/// malformed response from being buffered unbounded via `response.json()`.
const MAX_RELEASE_JSON_BYTES: u64 = 1024 * 1024;

/// Downloads `url`'s body, rejecting it the moment the accumulated length
/// would exceed `limit` rather than after buffering the whole thing. A
/// `Content-Length` over the limit is rejected as a fast path, but is not
/// relied on alone since it is attacker-controlled and may be absent.
async fn fetch_bounded(
    http: &reqwest::Client,
    url: String,
    limit: u64,
    what: &str,
) -> Result<Vec<u8>> {
    let response = http.get(url).send().await?;
    bound_body(response, limit, what).await
}

/// The bounded-read half of `fetch_bounded`, split out so callers that must
/// inspect the response (e.g. its status code) before deciding to buffer the
/// body can still get the same length enforcement.
async fn bound_body(mut response: reqwest::Response, limit: u64, what: &str) -> Result<Vec<u8>> {
    if let Some(len) = response.content_length() {
        if len > limit {
            return Err(BbError::Config(format!(
                "{what} reports {len} bytes, larger than the {limit} byte limit"
            )));
        }
    }
    let mut buf = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        buf.extend_from_slice(&chunk);
        if buf.len() as u64 > limit {
            return Err(BbError::Config(format!(
                "{what} exceeded the {limit} byte limit"
            )));
        }
    }
    Ok(buf)
}

pub fn release_api_base() -> String {
    std::env::var("BB_UPDATE_API_BASE").unwrap_or_else(|_| DEFAULT_RELEASE_API.to_string())
}

pub async fn run(format: Format, base_url: &str) -> Result<()> {
    let current = env!("CARGO_PKG_VERSION");
    let http = release_client()?;
    let url = format!(
        "{}/repos/biokraft/bbcloud/releases/latest",
        base_url.trim_end_matches('/')
    );
    let response = http.get(&url).send().await?;
    if !response.status().is_success() {
        return Err(BbError::Api {
            status: response.status().as_u16(),
            message: "cannot reach the release api".into(),
        });
    }
    let body = bound_body(response, MAX_RELEASE_JSON_BYTES, "release metadata").await?;
    let release: Release = serde_json::from_slice(&body)?;
    let latest = release.tag_name.clone();

    if !is_newer(&latest, current) {
        return report(format, current, &latest, true, "none");
    }

    let exe = std::env::current_exe().map_err(BbError::Io)?;
    let action = match classify_install(&exe) {
        InstallKind::Homebrew => "brew upgrade bb",
        InstallKind::Cargo => "cargo install bbcloud --locked --force",
        InstallKind::Standalone => {
            // The https-only requirement below is scoped to real usage: it
            // only applies when the release api itself is https (the
            // production default). The `BB_UPDATE_API_BASE` test override
            // that points at a local http wiremock server also relaxes the
            // asset-url check, so integration tests can exercise the
            // download-and-unpack path without standing up TLS.
            let require_https = base_url.starts_with("https://");
            self_update(&http, &release, &exe, require_https).await?;
            "self-updated"
        }
    };
    report(format, current, &latest, false, action)
}

fn report(
    format: Format,
    current: &str,
    latest: &str,
    up_to_date: bool,
    action: &str,
) -> Result<()> {
    match format {
        Format::Json => output::print_json(&serde_json::json!({
            "current": current,
            "latest": latest,
            "up_to_date": up_to_date,
            "action": action,
        })),
        Format::Human => {
            if up_to_date {
                output::success(&format!("bb {current} is up to date"));
            } else {
                output::info(&format!("{current} -> {latest}"));
                if action == "self-updated" {
                    output::success("updated in place");
                } else {
                    output::info(&format!("this install is managed elsewhere; run: {action}"));
                }
            }
            Ok(())
        }
    }
}

/// Removes the staged file on drop unless disarmed. This guarantees cleanup
/// on every early-return failure path after the file is created, not just
/// the one case that used to check for it.
struct StagedGuard {
    path: std::path::PathBuf,
    armed: bool,
}

impl StagedGuard {
    fn new(path: std::path::PathBuf) -> Self {
        Self { path, armed: true }
    }

    /// Call once the file has been successfully renamed into place, so drop
    /// does not try to remove a path that is now the live binary (or that no
    /// longer exists).
    fn disarm(mut self) {
        self.armed = false;
    }
}

impl Drop for StagedGuard {
    fn drop(&mut self) {
        if self.armed {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// The url comes from the release payload; a hijacked payload must not be
/// able to downgrade the transport used to fetch the binary. `require_https`
/// is false only for the `BB_UPDATE_API_BASE` test override, so production
/// use (the default `https://api.github.com`) always enforces this.
fn checked_asset_url(name: &str, url: String, require_https: bool) -> Result<String> {
    if require_https && !url.starts_with("https://") {
        return Err(BbError::Config(format!(
            "release asset {name} has a non-https download url"
        )));
    }
    Ok(url)
}

/// Downloads, verifies, unpacks and atomically replaces the running binary.
/// Nothing is written next to the binary until the digest matches.
async fn self_update(
    http: &reqwest::Client,
    release: &Release,
    exe: &Path,
    require_https: bool,
) -> Result<()> {
    let triple = current_triple()
        .ok_or_else(|| BbError::Config("no published binary for this platform".into()))?;
    let (archive_name, checksum_name) = asset_names(&release.tag_name, triple);

    let find = |name: &str| -> Result<String> {
        let url = release
            .assets
            .iter()
            .find(|a| a.name == name)
            .map(|a| a.browser_download_url.clone())
            .ok_or_else(|| BbError::Config(format!("release asset {name} is missing")))?;
        checked_asset_url(name, url, require_https)
    };

    let archive_bytes = fetch_bounded(
        http,
        find(&archive_name)?,
        MAX_ARCHIVE_BYTES,
        "release archive",
    )
    .await?;
    let checksum_bytes = fetch_bounded(
        http,
        find(&checksum_name)?,
        MAX_CHECKSUM_BYTES,
        "checksum file",
    )
    .await?;
    let expected = String::from_utf8_lossy(&checksum_bytes);
    let expected = expected
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_lowercase();

    use sha2::{Digest, Sha256};
    let actual = format!("{:x}", Sha256::digest(&archive_bytes));
    if actual != expected {
        return Err(BbError::Config(
            "checksum mismatch — refusing to install this download".into(),
        ));
    }

    let parent = exe
        .parent()
        .ok_or_else(|| BbError::Config("cannot determine the install directory".into()))?;

    // Entropy in the name defeats a pre-planted symlink at a fixed path and
    // avoids two concurrent `bb update` runs colliding on the same file.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let staged = parent.join(format!(".bb-update-staged-{}-{now}", std::process::id()));

    let mut found = false;
    let decoder = flate2::read::GzDecoder::new(std::io::Cursor::new(&archive_bytes[..]));
    let mut archive = tar::Archive::new(decoder);
    for entry in archive.entries()? {
        let mut entry = entry?;
        let is_bb = entry
            .path()?
            .file_name()
            .map(|n| n == std::ffi::OsStr::new("bb"))
            .unwrap_or(false);
        if !is_bb {
            continue;
        }
        // Do not let tar decide the node type. A symlink or hard-link entry
        // named `bb` must never be followed: `Entry::unpack` skips link
        // validation when given an explicit destination with no
        // `target_base`, which lets a malicious archive chmod or overwrite
        // an arbitrary file outside the install directory. Only a plain
        // regular file is accepted; anything else is treated as "not found".
        if !entry.header().entry_type().is_file() {
            continue;
        }

        // create_new(true) fails if the path already exists, which also
        // closes the pre-planted-file/symlink hole at the staged path.
        let mut out = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&staged)
            .map_err(BbError::Io)?;
        let guard = StagedGuard::new(staged.clone());

        let mut limited = std::io::Read::take(&mut entry, MAX_UNPACKED_BYTES);
        let copied = std::io::copy(&mut limited, &mut out).map_err(BbError::Io)?;
        drop(out);
        if copied >= MAX_UNPACKED_BYTES {
            return Err(BbError::Config(format!(
                "unpacked bb binary exceeds the {MAX_UNPACKED_BYTES} byte limit"
            )));
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755))
                .map_err(BbError::Io)?;
        }

        // Same-directory rename is atomic, so an interrupted update can
        // never leave a truncated `bb` behind.
        std::fs::rename(&staged, exe).map_err(BbError::Io)?;
        guard.disarm();
        found = true;
        break;
    }
    if !found {
        return Err(BbError::Config(
            "archive contains no regular-file bb binary".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn homebrew_paths_are_detected() {
        for p in [
            "/opt/homebrew/bin/bb",
            "/usr/local/Cellar/bb/1.0.0/bin/bb",
            "/home/linuxbrew/.linuxbrew/bin/bb",
        ] {
            assert_eq!(classify_install(Path::new(p)), InstallKind::Homebrew, "{p}");
        }
    }

    #[test]
    fn cargo_bin_is_detected() {
        assert_eq!(
            classify_install(Path::new("/Users/dev/.cargo/bin/bb")),
            InstallKind::Cargo
        );
    }

    #[test]
    fn anything_else_is_standalone() {
        for p in ["/usr/local/bin/bb", "/home/dev/.local/bin/bb", "./bb"] {
            assert_eq!(
                classify_install(Path::new(p)),
                InstallKind::Standalone,
                "{p}"
            );
        }
    }

    #[test]
    fn versions_parse_with_and_without_a_v_prefix() {
        assert_eq!(parse_version("1.2.3"), Some((1, 2, 3)));
        assert_eq!(parse_version("v1.2.3"), Some((1, 2, 3)));
        assert_eq!(parse_version("v10.0.1"), Some((10, 0, 1)));
    }

    #[test]
    fn malformed_versions_are_rejected_rather_than_panicking() {
        for bad in ["", "v", "1.2", "1.2.x", "latest", "v1.2.3.4"] {
            assert_eq!(parse_version(bad), None, "{bad}");
        }
    }

    #[test]
    fn is_newer_compares_each_component() {
        assert!(is_newer("v1.0.1", "1.0.0"));
        assert!(is_newer("v1.1.0", "1.0.9"));
        assert!(is_newer("v2.0.0", "1.9.9"));
        assert!(!is_newer("v1.0.0", "1.0.0"));
        assert!(!is_newer("v0.9.0", "1.0.0"));
    }

    /// An unparseable remote tag must never be treated as an upgrade — that
    /// would download and install an arbitrary asset on a malformed response.
    #[test]
    fn unparseable_remote_tag_is_not_newer() {
        assert!(!is_newer("garbage", "1.0.0"));
        assert!(!is_newer("", "1.0.0"));
    }

    #[test]
    fn https_asset_urls_are_required_when_enforced() {
        assert!(
            checked_asset_url("bb.tar.gz", "http://evil.example/bb.tar.gz".into(), true).is_err()
        );
        assert!(
            checked_asset_url("bb.tar.gz", "https://example.com/bb.tar.gz".into(), true).is_ok()
        );
    }

    #[test]
    fn https_enforcement_is_skipped_for_the_test_override() {
        assert!(
            checked_asset_url("bb.tar.gz", "http://127.0.0.1:1234/bb.tar.gz".into(), false).is_ok()
        );
    }

    #[test]
    fn asset_names_follow_the_release_workflow_convention() {
        let (archive, checksum) = asset_names("v1.0.0", "x86_64-apple-darwin");
        assert_eq!(archive, "bbcloud-v1.0.0-x86_64-apple-darwin.tar.gz");
        assert_eq!(checksum, "bbcloud-v1.0.0-x86_64-apple-darwin.tar.gz.sha256");
    }
}
