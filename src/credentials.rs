use crate::error::{BbError, Result};
use crate::secret::{ExposeSecret, SecretString};
use std::path::PathBuf;

const KEYRING_SERVICE: &str = "bb-cli";
const KEYRING_USER: &str = "bitbucket-api-token";

/// Email plus API token. `Debug` deliberately omits the token.
#[derive(Clone)]
pub struct Credentials {
    pub email: String,
    pub token: SecretString,
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credentials")
            .field("email", &self.email)
            .field("token", &"<redacted>")
            .finish()
    }
}

impl Credentials {
    pub fn basic_header(&self) -> SecretString {
        let raw = format!("{}:{}", self.email, self.token.expose_secret());
        SecretString::from(format!("Basic {}", base64_encode(raw.as_bytes())))
    }

    /// Safe-to-print form of the token.
    pub fn redacted_token(&self) -> String {
        crate::secret::redact(self.token.expose_secret())
    }
}

fn base64_encode(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            TABLE[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

pub fn keyring_entry() -> Option<keyring::Entry> {
    keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER).ok()
}

fn keyring_email_entry() -> Option<keyring::Entry> {
    keyring::Entry::new(KEYRING_SERVICE, "bitbucket-email").ok()
}

pub fn load() -> Result<Credentials> {
    let env_email = std::env::var("BB_EMAIL")
        .ok()
        .filter(|v| !v.trim().is_empty());
    let env_token = std::env::var("BB_TOKEN")
        .ok()
        .filter(|v| !v.trim().is_empty());
    if let (Some(email), Some(token)) = (env_email, env_token) {
        return Ok(Credentials {
            email,
            token: SecretString::from(token),
        });
    }

    if std::env::var("BB_KEYRING_DISABLE").is_ok() {
        return Err(BbError::Auth);
    }

    let token = keyring_entry().and_then(|e| e.get_password().ok());
    let email = keyring_email_entry().and_then(|e| e.get_password().ok());
    match (email, token) {
        (Some(email), Some(token)) => Ok(Credentials {
            email,
            token: SecretString::from(token),
        }),
        _ => Err(BbError::Auth),
    }
}

pub fn store(email: &str, token: &SecretString) -> Result<()> {
    if std::env::var("BB_KEYRING_DISABLE").is_ok() {
        return Ok(());
    }

    let token_entry =
        keyring_entry().ok_or_else(|| BbError::Config("cannot open os keyring".into()))?;
    let email_entry =
        keyring_email_entry().ok_or_else(|| BbError::Config("cannot open os keyring".into()))?;
    token_entry
        .set_password(token.expose_secret())
        .map_err(|e| BbError::Config(format!("cannot write token to keyring: {e}")))?;
    email_entry
        .set_password(email)
        .map_err(|e| BbError::Config(format!("cannot write email to keyring: {e}")))?;
    Ok(())
}

pub fn delete() -> Result<()> {
    if std::env::var("BB_KEYRING_DISABLE").is_ok() {
        return Ok(());
    }

    for entry in [keyring_entry(), keyring_email_entry()]
        .into_iter()
        .flatten()
    {
        // A missing entry is not an error for `logout`.
        let _ = entry.delete_credential();
    }
    Ok(())
}

pub fn legacy_config_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    PathBuf::from(home).join(".bitbucket-rest-cli-config.json")
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use secrecy::SecretString;
    use serial_test::serial;

    #[test]
    fn basic_header_encodes_email_and_token() {
        let creds = Credentials {
            email: "dev@example.com".into(),
            token: SecretString::from("s3cr3t"),
        };
        // base64("dev@example.com:s3cr3t")
        assert_eq!(
            secrecy::ExposeSecret::expose_secret(&creds.basic_header()),
            "Basic ZGV2QGV4YW1wbGUuY29tOnMzY3IzdA=="
        );
    }

    #[test]
    #[serial]
    fn env_vars_take_precedence_over_keyring() {
        std::env::set_var("BB_EMAIL", "env@example.com");
        std::env::set_var("BB_TOKEN", "envtoken");
        let creds = load().unwrap();
        assert_eq!(creds.email, "env@example.com");
        std::env::remove_var("BB_EMAIL");
        std::env::remove_var("BB_TOKEN");
    }

    #[test]
    #[serial]
    fn missing_credentials_yield_auth_error() {
        std::env::remove_var("BB_EMAIL");
        std::env::remove_var("BB_TOKEN");
        // `BB_KEYRING_DISABLE` short-circuits the keyring lookup, so this asserts
        // unconditionally instead of depending on whether the machine running the
        // test happens to have a stored entry.
        std::env::set_var("BB_KEYRING_DISABLE", "1");
        let result = load();
        std::env::remove_var("BB_KEYRING_DISABLE");
        assert!(matches!(result, Err(BbError::Auth)), "expected Auth error");
    }

    /// A credential builder that panics the instant anything tries to construct an
    /// `Entry` through it. Stands in for "the real OS keyring" for this test: keyring's
    /// mock store gives each `Entry::new` call independent, unshared storage (see
    /// `CredentialPersistence::EntryOnly` in `keyring::mock`), so it cannot prove
    /// `delete()`'s *internal* entries were never touched — this builder can, because
    /// it fires on construction itself, before any get/set/delete call.
    struct PanicOnConstruction;

    impl keyring::credential::CredentialBuilderApi for PanicOnConstruction {
        fn build(
            &self,
            _target: Option<&str>,
            _service: &str,
            _user: &str,
        ) -> keyring::Result<Box<keyring::credential::Credential>> {
            panic!(
                "delete() constructed a keyring Entry despite BB_KEYRING_DISABLE being set; \
                 it must return before touching the credential store at all"
            );
        }

        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
    }

    #[test]
    #[serial]
    fn delete_with_keyring_disabled_never_touches_the_credential_store() {
        // **IMPORTANT: Global State Mutation (process-wide, no cleanup)**
        // This test installs a panicking credential builder as the process-global default
        // via `keyring::set_default_credential_builder()`. The keyring crate does not
        // provide a public API to retrieve or restore the previous builder, so this
        // mutation persists for the entire remainder of the test binary.
        //
        // Any future test that exercises the real keyring path (i.e., one that calls
        // `keyring_entry()` or `keyring_email_entry()` when BB_KEYRING_DISABLE is unset)
        // will panic if it runs after this test. To avoid this:
        //
        // 1. Ensure any test needing real keyring access runs BEFORE this test, OR
        // 2. Ensure such tests account for the panicking builder being installed, OR
        // 3. Run this test last (e.g., via a separate test suite or final phase).
        //
        // The test validates that `delete()` respects BB_KEYRING_DISABLE by confirming
        // the builder never gets instantiated (it would panic if it did). This is the only
        // reliable way to prove `delete()` returns early and never touches the keyring.
        keyring::set_default_credential_builder(Box::new(PanicOnConstruction));

        std::env::set_var("BB_KEYRING_DISABLE", "1");
        let result = delete();
        std::env::remove_var("BB_KEYRING_DISABLE");

        assert!(result.is_ok(), "delete() should still report success");
    }

    #[test]
    #[serial]
    fn store_with_keyring_disabled_never_touches_the_credential_store() {
        // Mirrors `delete_with_keyring_disabled_never_touches_the_credential_store` above:
        // installs the same panicking builder (idempotent if already installed by that
        // test) and proves `store()` returns before constructing any keyring `Entry`.
        keyring::set_default_credential_builder(Box::new(PanicOnConstruction));

        std::env::set_var("BB_KEYRING_DISABLE", "1");
        let result = store(
            "dev@example.com",
            &SecretString::from("s3cr3t-should-never-reach-the-keyring"),
        );
        std::env::remove_var("BB_KEYRING_DISABLE");

        assert!(result.is_ok(), "store() should still report success");
    }

    #[test]
    fn debug_impl_renders_exactly_the_redacted_shape() {
        let creds = Credentials {
            email: "dev@example.com".into(),
            token: SecretString::from("ATATT_leaky_value"),
        };
        let shown = format!("{creds:?}");

        // Pinned exactly: a `#[derive(Debug)]` would render the SecretString's own
        // Debug (`SecretBox<..>`) instead of this, so this test fails if the
        // hand-written impl is removed.
        assert_eq!(
            shown,
            r#"Credentials { email: "dev@example.com", token: "<redacted>" }"#
        );
        assert!(!shown.contains("leaky"), "token leaked: {shown}");
    }
}
