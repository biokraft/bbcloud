#![allow(clippy::unwrap_used)] // test code is exempt from the unwrap/expect ban

use assert_cmd::Command;
use predicates::str::contains;

#[test]
fn prints_version() {
    Command::cargo_bin("bb")
        .unwrap()
        .arg("--version")
        .assert()
        .success()
        .stdout(contains(env!("CARGO_PKG_VERSION")));
}

#[test]
fn help_lists_every_top_level_command() {
    let out = Command::cargo_bin("bb")
        .unwrap()
        .arg("--help")
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    for expected in ["auth", "pr", "branch", "browse", "completions"] {
        assert!(
            text.contains(expected),
            "missing `{expected}` in help:\n{text}"
        );
    }
}

#[test]
fn pr_help_lists_read_and_write_commands() {
    let out = Command::cargo_bin("bb")
        .unwrap()
        .args(["pr", "--help"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    for expected in [
        "list", "view", "create", "comment", "diff", "files", "commits",
    ] {
        assert!(
            text.contains(expected),
            "missing `pr {expected}` in help:\n{text}"
        );
    }
}

#[test]
fn no_php_sources_remain() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    for stale in [
        "bin/bb",
        "src/Base.php",
        "src/Actions",
        "create-phar.php",
        "config/app.php",
    ] {
        assert!(
            !root.join(stale).exists(),
            "{stale} should have been deleted"
        );
    }
}

#[test]
fn help_does_not_mention_app_passwords() {
    let out = Command::cargo_bin("bb")
        .unwrap()
        .arg("--help")
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout).to_lowercase();
    assert!(
        !text.contains("app password"),
        "help must not advertise removed app passwords"
    );
}
