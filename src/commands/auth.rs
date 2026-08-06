use crate::api::Client;
use crate::credentials::{self, Credentials};
use crate::error::{BbError, Result};
use crate::output::{self, Format};
use crate::secret::SecretString;
use serde::Serialize;

const TOKEN_HELP_URL: &str = "https://id.atlassian.com/manage-profile/security/api-tokens";

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
    if !format.is_json() {
        output::info("bb authenticates with an atlassian api token");
        output::info(&format!("create one at {TOKEN_HELP_URL}"));
    }

    // Never block on input that will not arrive: if stdin is not a terminal and
    // either value would require a prompt, name the flags instead of hanging.
    let would_prompt = email.is_none() || !token_stdin;
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
    let user: crate::api::models::User = client.get_json("/user").await?;
    spinner.finish_and_clear();

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
