#![allow(clippy::unwrap_used)] // test code is exempt from the unwrap/expect ban

use assert_cmd::Command;
use predicates::str::contains;

/// The token must never appear in `bb auth status` output, in either format.
#[test]
fn auth_status_redacts_the_token() {
    for args in [vec!["auth", "status"], vec!["auth", "status", "--json"]] {
        let out = Command::cargo_bin("bb")
            .unwrap()
            .args(&args)
            .env("BB_EMAIL", "dev@example.com")
            .env("BB_TOKEN", "ATATT3xFfGF0_super_secret_value")
            .env("BB_API_BASE", "http://127.0.0.1:1")
            .output()
            .unwrap();

        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            !combined.contains("ATATT3xFfGF0"),
            "token leaked for {args:?}: {combined}"
        );
        assert!(
            !combined.contains("super_secret"),
            "token body leaked for {args:?}: {combined}"
        );
    }
}

#[test]
fn auth_status_shows_email_and_redacted_tail() {
    Command::cargo_bin("bb")
        .unwrap()
        .args(["auth", "status"])
        .env("BB_EMAIL", "dev@example.com")
        .env("BB_TOKEN", "ATATT3xFfGF0abcd")
        .env("BB_API_BASE", "http://127.0.0.1:1")
        .assert()
        .stdout(contains("dev@example.com"))
        .stdout(contains("****abcd"));
}

#[test]
fn auth_status_without_credentials_exits_two() {
    Command::cargo_bin("bb")
        .unwrap()
        .args(["auth", "status"])
        .env("BB_EMAIL", "")
        .env("BB_TOKEN", "")
        .env("BB_KEYRING_DISABLE", "1")
        .assert()
        .code(2)
        .stderr(contains("bb auth login"));
}

/// `--json` must emit parseable JSON on stdout, not the human success line.
#[test]
fn auth_logout_json_emits_parseable_json() {
    let out = Command::cargo_bin("bb")
        .unwrap()
        .args(["auth", "logout", "--json"])
        .env("BB_KEYRING_DISABLE", "1")
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    let _value: serde_json::Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("stdout did not parse as JSON: {e}\nstdout: {stdout}"));
}

/// Non-interactive `auth login` without --email/--token-stdin must name both flags
/// rather than hang waiting on a prompt.
#[test]
fn auth_login_non_tty_names_required_flags() {
    let assert = Command::cargo_bin("bb")
        .unwrap()
        .args(["auth", "login"])
        .env("BB_API_BASE", "http://127.0.0.1:1")
        .write_stdin("")
        .assert()
        .failure();
    let output = assert.get_output();
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(combined.contains("--email"), "missing --email: {combined}");
    assert!(
        combined.contains("--token-stdin"),
        "missing --token-stdin: {combined}"
    );
}

/// The secret must never leak, in either human or --json mode, even on this error path.
#[test]
fn auth_login_non_tty_never_leaks_secret() {
    for args in [vec!["auth", "login"], vec!["auth", "login", "--json"]] {
        let out = Command::cargo_bin("bb")
            .unwrap()
            .args(&args)
            .env("BB_API_BASE", "http://127.0.0.1:1")
            .write_stdin("super-secret-token-value")
            .output()
            .unwrap();
        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            !combined.contains("super-secret-token-value"),
            "secret leaked for {args:?}: {combined}"
        );
    }
}

#[test]
fn auth_help_mentions_api_token_not_app_password() {
    let out = Command::cargo_bin("bb")
        .unwrap()
        .args(["auth", "login", "--help"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout).to_lowercase();
    assert!(text.contains("api token"));
    assert!(!text.contains("app password"));
}
