#![allow(clippy::unwrap_used)] // test code is exempt from the unwrap/expect ban

use assert_cmd::Command;
use predicates::str::contains;
use tempfile::tempdir;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// The token must never appear in `bb auth status` output, in either format.
#[test]
fn auth_status_redacts_the_token() {
    for args in [vec!["auth", "status"], vec!["auth", "status", "--json"]] {
        let out = Command::cargo_bin("bb")
            .unwrap()
            .env("BB_NO_UPDATE_CHECK", "1")
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
        .env("BB_NO_UPDATE_CHECK", "1")
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
        .env("BB_NO_UPDATE_CHECK", "1")
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
        .env("BB_NO_UPDATE_CHECK", "1")
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
        .env("BB_NO_UPDATE_CHECK", "1")
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
            .env("BB_NO_UPDATE_CHECK", "1")
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
        .env("BB_NO_UPDATE_CHECK", "1")
        .args(["auth", "login", "--help"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout).to_lowercase();
    assert!(text.contains("api token"));
    assert!(!text.contains("app password"));
}

/// `login` with --email and --token-stdin never prompts, so the whole verify-then-store
/// path runs without a tty.
#[tokio::test]
async fn login_verifies_the_token_and_reports_the_account() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/user"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"display_name": "Dev Person"})),
        )
        .mount(&server)
        .await;

    let out = Command::cargo_bin("bb")
        .unwrap()
        .env("BB_NO_UPDATE_CHECK", "1")
        .args([
            "auth",
            "login",
            "--email",
            "dev@example.com",
            "--token-stdin",
        ])
        .env("BB_API_BASE", server.uri())
        .env("BB_KEYRING_DISABLE", "1")
        .env("NO_COLOR", "1")
        .env_remove("BB_EMAIL")
        .env_remove("BB_TOKEN")
        .write_stdin("ATATT3xFfGF0abcd")
        .output()
        .unwrap();

    assert!(out.status.success(), "login failed: {out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("Dev Person"), "no account name: {stdout}");
    assert!(stdout.contains("****abcd"), "no redacted tail: {stdout}");
    assert!(
        !stdout.contains("ATATT3xFfGF0abcd"),
        "token leaked: {stdout}"
    );
}

/// The same path in --json mode must emit pure JSON on stdout and no human lines.
#[tokio::test]
async fn login_json_emits_pure_json() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/user"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"display_name": "Dev Person"})),
        )
        .mount(&server)
        .await;

    let out = Command::cargo_bin("bb")
        .unwrap()
        .env("BB_NO_UPDATE_CHECK", "1")
        .args([
            "auth",
            "login",
            "--email",
            "dev@example.com",
            "--token-stdin",
            "--json",
        ])
        .env("BB_API_BASE", server.uri())
        .env("BB_KEYRING_DISABLE", "1")
        .env("NO_COLOR", "1")
        .env_remove("BB_EMAIL")
        .env_remove("BB_TOKEN")
        .write_stdin("ATATT3xFfGF0abcd")
        .output()
        .unwrap();

    assert!(out.status.success(), "login failed: {out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let value: serde_json::Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("stdout was not pure JSON: {e}\nstdout: {stdout}"));
    assert_eq!(value["account"], "Dev Person");
    assert_eq!(value["email"], "dev@example.com");
    assert_eq!(value["token"], "****abcd");
}

/// A value that is not an email address is rejected before any network call.
#[test]
fn login_rejects_an_email_without_an_at_sign() {
    Command::cargo_bin("bb")
        .unwrap()
        .env("BB_NO_UPDATE_CHECK", "1")
        .args(["auth", "login", "--email", "not-an-email", "--token-stdin"])
        .env("BB_API_BASE", "http://127.0.0.1:1")
        .env("BB_KEYRING_DISABLE", "1")
        .env("NO_COLOR", "1")
        .env_remove("BB_EMAIL")
        .env_remove("BB_TOKEN")
        .write_stdin("some-token")
        .assert()
        .failure()
        .stderr(contains("atlassian account email"));
}

/// A leftover plaintext credential file from the PHP-era CLI must be called out on logout.
#[test]
fn logout_warns_about_a_legacy_plaintext_credential_file() {
    let home = tempdir().unwrap();
    std::fs::write(
        home.path().join(".bitbucket-rest-cli-config.json"),
        "{\"token\":\"whatever\"}",
    )
    .unwrap();

    Command::cargo_bin("bb")
        .unwrap()
        .env("BB_NO_UPDATE_CHECK", "1")
        .args(["auth", "logout"])
        .env("HOME", home.path())
        .env("BB_KEYRING_DISABLE", "1")
        .env("NO_COLOR", "1")
        .assert()
        .success()
        .stderr(contains("legacy plaintext credential file"));
}

/// A failing identity check must not fail the command — the account is simply unknown.
#[tokio::test]
async fn status_reports_an_unverified_account_when_the_identity_check_fails() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/user"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let out = Command::cargo_bin("bb")
        .unwrap()
        .env("BB_NO_UPDATE_CHECK", "1")
        .args(["auth", "status", "--json"])
        .env("BB_EMAIL", "dev@example.com")
        .env("BB_TOKEN", "ATATT3xFfGF0abcd")
        .env("BB_API_BASE", server.uri())
        .env("BB_KEYRING_DISABLE", "1")
        .output()
        .unwrap();

    assert!(out.status.success(), "status failed: {out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let value: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert!(
        value["account"].is_null(),
        "expected a null account, got {value}"
    );
}

/// Onboarding: a user who is about to be prompted has to be told where to create
/// the token and which scopes to grant, otherwise verification fails and they
/// cannot guess which of Bitbucket's scopes this tool wanted.
#[test]
fn login_prints_the_token_url_and_every_required_scope() {
    let out = Command::cargo_bin("bb")
        .unwrap()
        .env("BB_NO_UPDATE_CHECK", "1")
        .args(["auth", "login"])
        .env("BB_API_BASE", "http://127.0.0.1:1")
        .env("BB_KEYRING_DISABLE", "1")
        .env("NO_COLOR", "1")
        .env_remove("BB_EMAIL")
        .env_remove("BB_TOKEN")
        .write_stdin("")
        .output()
        .unwrap();

    assert_eq!(
        out.status.code(),
        Some(1),
        "expected the non-interactive config error: {out:?}"
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("https://id.atlassian.com/manage-profile/security/api-tokens"),
        "no token url: {stdout}"
    );
    for (scope, _) in bb_cli::commands::auth::SCOPES {
        assert!(stdout.contains(scope), "scope {scope} missing: {stdout}");
    }
    assert!(
        stdout.contains("Create API token with scopes"),
        "no scoped-token instruction: {stdout}"
    );
}

/// The walkthrough is for someone who will type values. `--email` with
/// `--token-stdin` prompts for nothing, so on a CI runner the lines would be noise
/// in the captured log.
#[tokio::test]
async fn login_with_both_values_supplied_prints_no_walkthrough() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/user"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"display_name": "Dev Person"})),
        )
        .mount(&server)
        .await;

    let out = Command::cargo_bin("bb")
        .unwrap()
        .env("BB_NO_UPDATE_CHECK", "1")
        .args([
            "auth",
            "login",
            "--email",
            "dev@example.com",
            "--token-stdin",
        ])
        .env("BB_API_BASE", server.uri())
        .env("BB_KEYRING_DISABLE", "1")
        .env("NO_COLOR", "1")
        .env_remove("BB_EMAIL")
        .env_remove("BB_TOKEN")
        .write_stdin("ATATT3xFfGF0abcd")
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains("Create API token with scopes"),
        "walkthrough printed on the non-interactive path: {stdout}"
    );
}

/// `--help` carries the same guidance, because `--email`/`--token-stdin` skips the
/// interactive walkthrough entirely. Driven off `SCOPES`, so the clap `long_about`
/// cannot drift away from the list the walkthrough prints.
#[test]
fn auth_login_help_lists_the_required_scopes() {
    let out = Command::cargo_bin("bb")
        .unwrap()
        .env("BB_NO_UPDATE_CHECK", "1")
        .args(["auth", "login", "--help"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("id.atlassian.com"), "no token url: {text}");
    for (scope, _) in bb_cli::commands::auth::SCOPES {
        assert!(text.contains(scope), "scope {scope} missing: {text}");
    }
}

/// A 403 on the verification call means the token exists but lacks
/// `read:user:bitbucket`. Saying so beats the generic scope wording `check()` emits.
#[tokio::test]
async fn login_names_the_missing_user_scope_on_a_403() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/user"))
        .respond_with(
            ResponseTemplate::new(403)
                .set_body_json(serde_json::json!({"error": {"message": "Forbidden"}})),
        )
        .mount(&server)
        .await;

    let out = Command::cargo_bin("bb")
        .unwrap()
        .env("BB_NO_UPDATE_CHECK", "1")
        .args([
            "auth",
            "login",
            "--email",
            "dev@example.com",
            "--token-stdin",
        ])
        .env("BB_API_BASE", server.uri())
        .env("BB_KEYRING_DISABLE", "1")
        .env("NO_COLOR", "1")
        .env_remove("BB_EMAIL")
        .env_remove("BB_TOKEN")
        .write_stdin("ATATT3xFfGF0abcd")
        .output()
        .unwrap();

    assert!(!out.status.success(), "403 must fail login: {out:?}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("read:user:bitbucket"),
        "no scope hint: {stderr}"
    );
    let combined = format!("{}{stderr}", String::from_utf8_lossy(&out.stdout));
    assert!(
        !combined.contains("ATATT3xFfGF0abcd"),
        "token leaked: {combined}"
    );
}

/// A 401 means the pair itself was rejected — the hint must point at the email and
/// the token, not at scopes, and must not print the secret.
#[tokio::test]
async fn login_explains_a_rejected_email_and_token_pair() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/user"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;

    let out = Command::cargo_bin("bb")
        .unwrap()
        .env("BB_NO_UPDATE_CHECK", "1")
        .args([
            "auth",
            "login",
            "--email",
            "dev@example.com",
            "--token-stdin",
        ])
        .env("BB_API_BASE", server.uri())
        .env("BB_KEYRING_DISABLE", "1")
        .env("NO_COLOR", "1")
        .env_remove("BB_EMAIL")
        .env_remove("BB_TOKEN")
        .write_stdin("ATATT3xFfGF0abcd")
        .output()
        .unwrap();

    assert_eq!(out.status.code(), Some(2), "401 must exit 2: {out:?}");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        combined.contains("atlassian account email"),
        "no email hint: {combined}"
    );
    assert!(
        !combined.contains("ATATT3xFfGF0abcd"),
        "token leaked: {combined}"
    );
}

/// The README's scope table and `SCOPES` are two hand-maintained lists of the
/// same facts, and they drifted: the table gained `read:project:bitbucket` and
/// `admin:repository:bitbucket` when `repo create` and `project list` shipped,
/// while the login walkthrough kept printing four scopes. Someone following
/// `bb auth login` then minted a token that 403s on the first `repo create`.
///
/// Every other scope test iterates `SCOPES`, so all of them stayed green through
/// that drift — they can only prove the walkthrough prints what the constant
/// holds. This one is the cross-check: it parses the README table and asserts
/// the two sets are equal, so adding a scope to either place alone fails here.
#[test]
fn the_readme_scope_table_matches_the_login_walkthrough() {
    let readme = include_str!("../README.md");
    let documented: Vec<&str> = readme
        .lines()
        .skip_while(|l| !l.starts_with("### Token scopes"))
        .take_while(|l| !l.starts_with("### CI"))
        .filter_map(|l| l.strip_prefix("| `"))
        .filter_map(|rest| rest.split('`').next())
        .collect();

    assert!(
        !documented.is_empty(),
        "found no scope rows — the README's `### Token scopes` table moved or changed shape"
    );

    let printed: Vec<&str> = bb_cli::commands::auth::SCOPES
        .iter()
        .map(|(s, _)| *s)
        .collect();

    for scope in &documented {
        assert!(
            printed.contains(scope),
            "README documents {scope} but `bb auth login` never tells anyone to grant it"
        );
    }
    for scope in &printed {
        assert!(
            documented.contains(scope),
            "`bb auth login` asks for {scope} but the README scope table does not list it"
        );
    }
}
