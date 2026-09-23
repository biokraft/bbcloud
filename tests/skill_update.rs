#![allow(clippy::unwrap_used)] // test code is exempt from the unwrap/expect ban

use assert_cmd::Command;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const OLD_SKILL: &str = "---\nname: bitbucket-cloud\n---\nold text\n";

fn release_body(tag: &str) -> serde_json::Value {
    serde_json::json!({ "tag_name": tag, "assets": [] })
}

/// Reports the running version as latest, so `update` takes the up-to-date path
/// and touches no binary.
async fn mock_up_to_date() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/biokraft/bbcloud/releases/latest"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(release_body(&format!("v{}", env!("CARGO_PKG_VERSION")))),
        )
        .mount(&server)
        .await;
    server
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

/// Writes a skill file plus a state entry claiming bb wrote exactly that text.
fn track(project: &std::path::Path, cfg: &std::path::Path, contents: &str) -> std::path::PathBuf {
    let file = project.join(".agents/skills/bitbucket-cloud/SKILL.md");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, contents).unwrap();

    let state = cfg.join("bb/skills.json");
    std::fs::create_dir_all(state.parent().unwrap()).unwrap();
    let entries = serde_json::json!([{
        "path": file,
        "agent": "agents",
        "kind": "file",
        "sha256": sha256_hex(contents.as_bytes()),
        "version": "0.0.1"
    }]);
    std::fs::write(&state, serde_json::to_string(&entries).unwrap()).unwrap();
    file
}

fn bb(project: &std::path::Path, cfg: &std::path::Path, api: &str) -> Command {
    let mut cmd = Command::cargo_bin("bb").unwrap();
    cmd.env("BB_NO_UPDATE_CHECK", "1");
    cmd.current_dir(project)
        .env("HOME", cfg)
        .env("XDG_CONFIG_HOME", cfg)
        .env("BB_UPDATE_API_BASE", api)
        .env("NO_COLOR", "1")
        .env("BB_KEYRING_DISABLE", "1");
    cmd
}

/// The point of tracking installs: a skill left behind by an older binary gets
/// brought up to date.
#[tokio::test]
async fn update_refreshes_a_stale_tracked_skill() {
    let server = mock_up_to_date().await;
    let project = tempfile::tempdir().unwrap();
    let cfg = tempfile::tempdir().unwrap();
    let file = track(project.path(), cfg.path(), OLD_SKILL);

    bb(project.path(), cfg.path(), &server.uri())
        .arg("update")
        .assert()
        .success();

    let now = std::fs::read_to_string(&file).unwrap();
    assert_ne!(now, OLD_SKILL, "stale skill was not refreshed");
    assert!(now.starts_with("---"), "refreshed file is not the skill");
    assert!(
        now.len() > OLD_SKILL.len(),
        "expected the embedded skill, got {now}"
    );
}

/// A customized skill survives untouched.
#[tokio::test]
async fn update_leaves_a_modified_skill_alone() {
    let server = mock_up_to_date().await;
    let project = tempfile::tempdir().unwrap();
    let cfg = tempfile::tempdir().unwrap();
    let file = track(project.path(), cfg.path(), OLD_SKILL);
    // Recorded hash now disagrees with what is on disk: a local edit.
    std::fs::write(&file, "# our own version\n").unwrap();

    bb(project.path(), cfg.path(), &server.uri())
        .arg("update")
        .assert()
        .success();

    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "# our own version\n",
        "a customized skill must not be overwritten by an upgrade"
    );
}

/// Keeping a local edit is right, but a silently stale skill describes a `bb`
/// that no longer exists after a release that added commands — so the skip has
/// to name the way out.
#[tokio::test]
async fn update_tells_you_how_to_take_the_new_version_of_a_modified_skill() {
    let server = mock_up_to_date().await;
    let project = tempfile::tempdir().unwrap();
    let cfg = tempfile::tempdir().unwrap();
    let file = track(project.path(), cfg.path(), OLD_SKILL);
    std::fs::write(&file, "# our own version\n").unwrap();

    let out = bb(project.path(), cfg.path(), &server.uri())
        .arg("update")
        .output()
        .unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );

    assert!(
        text.contains("skill install --force"),
        "the skip must name the remedy, got:\n{text}"
    );
}

/// Nothing tracked means nothing said about skills.
#[tokio::test]
async fn update_is_quiet_about_skills_when_none_are_tracked() {
    let server = mock_up_to_date().await;
    let project = tempfile::tempdir().unwrap();
    let cfg = tempfile::tempdir().unwrap();

    let out = bb(project.path(), cfg.path(), &server.uri())
        .arg("update")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.to_lowercase().contains("skill"),
        "should not mention skills when none are installed: {stdout}"
    );
}

/// `--json` stays pure even when the refresh has something to report.
#[tokio::test]
async fn update_json_stays_pure_while_refreshing() {
    let server = mock_up_to_date().await;
    let project = tempfile::tempdir().unwrap();
    let cfg = tempfile::tempdir().unwrap();
    track(project.path(), cfg.path(), OLD_SKILL);

    let out = bb(project.path(), cfg.path(), &server.uri())
        .args(["update", "--json"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let parsed: serde_json::Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("stdout is not pure json: {e}\n{stdout}"));
    assert_eq!(parsed["up_to_date"], serde_json::Value::Bool(true));
}
