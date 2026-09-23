use crate::api::Client;
use crate::credentials::{self, Credentials};
use crate::error::{BbError, Result};
use crate::output::{self, Format};
use crate::secret::SecretString;
use serde::Serialize;

const TOKEN_HELP_URL: &str = "https://id.atlassian.com/manage-profile/security/api-tokens";

/// The scopes `bb` needs, with what each one buys, in the order someone should
/// decide about them: `read:user:bitbucket` first because login itself fails
/// without it, then the read scopes a browsing user wants, then the grants that
/// only some commands need. Ordering is the only guidance a list this long can
/// give, since Atlassian's picker shows two dozen Bitbucket scopes at once.
///
/// This list and the README's "Token scopes" table are the same facts written
/// twice; `tests/auth.rs::the_readme_scope_table_matches_the_login_walkthrough`
/// asserts they stay equal, because they did not — the table gained the last
/// two entries while the walkthrough kept printing four.
pub const SCOPES: [(&str, &str); 6] = [
    (
        "read:user:bitbucket",
        "required — login verifies the token against /user",
    ),
    (
        "read:pullrequest:bitbucket",
        "pr list, view, diff, files, commits, mine",
    ),
    (
        "read:repository:bitbucket",
        "branch list, repo list, default reviewers, the pr mine scan",
    ),
    (
        "write:pullrequest:bitbucket",
        "pr create, comment, resolve, request-changes",
    ),
    (
        "read:project:bitbucket",
        "project list, and repo create's project picker",
    ),
    (
        "admin:repository:bitbucket",
        "repo create — no combination of the read scopes covers it",
    ),
];

/// The scope block both the walkthrough and `--help` print, one `scope  why`
/// line per entry, aligned on the widest name. Rendered from `SCOPES` rather
/// than written out a second time: the walkthrough, `--help` and the README
/// each used to carry their own copy of this list, and two of the three fell
/// behind when `repo create` shipped.
pub fn scope_lines(indent: &str) -> String {
    let width = SCOPES.iter().map(|(s, _)| s.len()).max().unwrap_or(0);
    SCOPES
        .iter()
        .map(|(scope, why)| format!("{indent}{scope:<width$}  {why}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// `--help` carries the same guidance as the interactive walkthrough, because
/// `--email` with `--token-stdin` skips the walkthrough entirely and a CI user
/// has nowhere else to read it.
pub fn login_long_about() -> String {
    format!(
        "Store an atlassian api token in the os keyring.

Create the token at {TOKEN_HELP_URL},
choosing \"Create API token with scopes\" and Bitbucket as the product, then grant:

{}

The first three are the read-only floor. Grant the rest only for the commands
named beside them: writing to a pull request, listing projects, and creating a
repository are each a separate grant.",
        scope_lines("  ")
    )
}

/// Printed before the prompts, because a token created without scopes — or with
/// the wrong ones — fails verification and the user has no way to guess which of
/// the two dozen Bitbucket scopes this tool wanted. Only for someone who is about
/// to type values: a caller that passed `--email` and `--token-stdin` already has
/// a token, and on a CI runner these lines are just noise in the captured log.
fn print_onboarding() {
    output::heading("bb authenticates with an atlassian api token");
    output::info(
        "atlassian retired the older bitbucket credential on 2026-07-28 — an api token is \
         the only one left",
    );
    println!();
    output::info(&format!("1. open {TOKEN_HELP_URL}"));
    output::info("2. choose \"Create API token with scopes\", then pick Bitbucket as the product");
    output::info("3. grant these scopes:");
    println!("{}", scope_lines("     "));
    output::info(
        "   the first three are the read-only floor; grant the rest only for the commands \
         beside them",
    );
    output::info("4. copy the token — atlassian shows it once — and paste it below");
    println!();
}

/// The likely cause of a failed verification, or `None` when the failure says
/// nothing about credentials. Printed as a warning rather than folded into the
/// error, so the exit code stays what the http layer decided: `check()` renders
/// every 401 as "not authenticated" and every 403 as a scope problem in general
/// terms, neither of which helps someone who has just typed a brand-new token.
fn verification_hint(err: &BbError) -> Option<&'static str> {
    match err {
        BbError::Auth => Some(
            "the email or token was rejected — the username must be your atlassian account \
             email, and the password the api token itself, not your atlassian password",
        ),
        BbError::Api { status: 403, .. } => Some(
            "the token was accepted but the request was refused — most likely the \
             read:user:bitbucket scope is missing; a revoked token or an organisation \
             access policy gives the same answer",
        ),
        _ => None,
    }
}

#[derive(Debug, Serialize)]
pub struct AuthStatus {
    pub email: String,
    /// Already redacted. Never holds the real token.
    pub token: String,
    pub account: Option<String>,
}

/// Renders an [`AuthStatus`] as either JSON or the `FIELD | VALUE` human table,
/// shared by `login` and `status` so they can't drift in shape.
fn print_status(format: Format, status: &AuthStatus, unverified_label: &str) -> Result<()> {
    match format {
        Format::Json => output::print_json(status),
        Format::Human => {
            output::print_table(
                &["FIELD", "VALUE"],
                vec![
                    vec!["email".into(), status.email.clone()],
                    vec!["token".into(), status.token.clone()],
                    vec![
                        "account".into(),
                        status
                            .account
                            .clone()
                            .unwrap_or_else(|| unverified_label.into()),
                    ],
                ],
            );
            Ok(())
        }
    }
}

pub async fn login(email: Option<String>, token_stdin: bool, format: Format) -> Result<()> {
    // Never block on input that will not arrive: if stdin is not a terminal and
    // either value would require a prompt, name the flags instead of hanging.
    let would_prompt = email.is_none() || !token_stdin;

    if would_prompt && !format.is_json() {
        print_onboarding();
    }

    if would_prompt && !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
        return Err(BbError::Config(
            "no email/token on a non-interactive stdin — pass --email and --token-stdin".into(),
        ));
    }

    let email = match email {
        Some(value) => value,
        None => inquire::Text::new("atlassian account email:")
            .prompt()
            .map_err(|e| BbError::Config(format!("cancelled: {e}")))?,
    };

    let token = if token_stdin {
        let mut buf = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut buf)?;
        SecretString::from(buf.trim().to_string())
    } else {
        // `Password` never echoes and never confirms into the terminal buffer.
        let entered = inquire::Password::new("api token:")
            .with_display_mode(inquire::PasswordDisplayMode::Masked)
            .without_confirmation()
            .prompt()
            .map_err(|e| BbError::Config(format!("cancelled: {e}")))?;
        SecretString::from(entered)
    };

    let email = email.trim().to_string();
    if email.is_empty() || !email.contains('@') {
        return Err(BbError::Config(
            "email must be the atlassian account email address".into(),
        ));
    }

    let creds = Credentials {
        email: email.clone(),
        token: token.clone(),
    };

    // Verify before persisting, so a bad token is never stored.
    let spinner = output::spinner("verifying token");
    let client = Client::from_env(creds.clone())?;
    let verified = client.get_json::<crate::api::models::User>("/user").await;
    spinner.finish_and_clear();
    let user = match verified {
        Ok(user) => user,
        Err(err) => {
            if let Some(hint) = verification_hint(&err) {
                output::warn(hint);
            }
            return Err(err);
        }
    };

    credentials::store(&email, &token)?;

    let status = AuthStatus {
        email,
        token: creds.redacted_token(),
        account: user.display_name,
    };

    if !format.is_json() {
        output::success("token verified and saved to the os keyring");
    }
    print_status(format, &status, "-")?;

    Ok(())
}

pub async fn status(format: Format) -> Result<()> {
    let creds = credentials::load()?;
    let redacted = creds.redacted_token();

    // Best-effort identity check; a network failure must not leak the token.
    let account = match Client::from_env(creds.clone()) {
        Ok(client) => client
            .get_json::<crate::api::models::User>("/user")
            .await
            .ok()
            .and_then(|u| u.display_name),
        Err(_) => None,
    };

    let status = AuthStatus {
        email: creds.email.clone(),
        token: redacted,
        account,
    };

    print_status(format, &status, "unverified")?;

    Ok(())
}

pub fn logout(format: Format) -> Result<()> {
    credentials::delete()?;
    let legacy = credentials::legacy_config_path();
    let legacy_exists = legacy.exists();

    match format {
        Format::Json => output::print_json(&serde_json::json!({ "removed": true }))?,
        Format::Human => {
            if legacy_exists {
                output::warn(&format!(
                    "a legacy plaintext credential file still exists at {} — delete it",
                    legacy.display()
                ));
            }
            output::success("credentials removed from the os keyring");
        }
    }
    Ok(())
}
