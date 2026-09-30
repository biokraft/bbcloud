use crate::error::{BbError, Result};
use crate::secret::{ExposeSecret, SecretString};
use std::path::PathBuf;

const KEYRING_SERVICE: &str = "bb-cli";
const KEYRING_USER: &str = "bitbucket-api-token";
const KEYRING_EMAIL_USER: &str = "bitbucket-email";

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

/// macOS goes through `/usr/bin/security` rather than the Security framework,
/// the way `gh` does. The keychain grants silent access to the app that created
/// an item, identified by its code signature; an ad-hoc signed `bb` gets a new
/// identity with every release, so each upgrade prompted for the login password.
/// Apple's signed `security` binary keeps its identity across `bb` upgrades.
#[cfg(target_os = "macos")]
mod store {
    use std::io::Write;
    use std::process::{Command, Stdio};

    const SECURITY: &str = "/usr/bin/security";

    pub fn get(service: &str, account: &str) -> Option<String> {
        let out = Command::new(SECURITY)
            .args(["find-generic-password", "-s", service, "-a", account, "-w"])
            .stderr(Stdio::null())
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let value = String::from_utf8(out.stdout).ok()?;
        Some(value.trim_end_matches('\n').to_string())
    }

    /// The secret travels hex-encoded over stdin, never in argv, so it stays out
    /// of `ps` and needs no quoting.
    pub fn set(service: &str, account: &str, secret: &str) -> Result<(), String> {
        let mut child = Command::new(SECURITY)
            .arg("-i")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| e.to_string())?;
        child
            .stdin
            .take()
            .ok_or("security has no stdin")?
            .write_all(add_command(service, account, secret).as_bytes())
            .map_err(|e| e.to_string())?;
        let out = child.wait_with_output().map_err(|e| e.to_string())?;
        if out.status.success() {
            Ok(())
        } else {
            Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
        }
    }

    pub fn delete(service: &str, account: &str) {
        let _ = Command::new(SECURITY)
            .args(["delete-generic-password", "-s", service, "-a", account])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }

    pub fn add_command(service: &str, account: &str, secret: &str) -> String {
        let hex: String = secret.bytes().map(|b| format!("{b:02x}")).collect();
        format!("add-generic-password -U -s {service} -a {account} -X {hex}\n")
    }
}

#[cfg(not(target_os = "macos"))]
mod store {
    pub fn get(service: &str, account: &str) -> Option<String> {
        keyring::Entry::new(service, account)
            .ok()?
            .get_password()
            .ok()
    }

    pub fn set(service: &str, account: &str, secret: &str) -> Result<(), String> {
        keyring::Entry::new(service, account)
            .and_then(|e| e.set_password(secret))
            .map_err(|e| e.to_string())
    }

    pub fn delete(service: &str, account: &str) {
        if let Ok(entry) = keyring::Entry::new(service, account) {
            let _ = entry.delete_credential();
        }
    }
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

    let token = store::get(KEYRING_SERVICE, KEYRING_USER);
    let email = store::get(KEYRING_SERVICE, KEYRING_EMAIL_USER);
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

    store::set(KEYRING_SERVICE, KEYRING_USER, token.expose_secret())
        .map_err(|e| BbError::Config(format!("cannot write token to keyring: {e}")))?;
    store::set(KEYRING_SERVICE, KEYRING_EMAIL_USER, email)
        .map_err(|e| BbError::Config(format!("cannot write email to keyring: {e}")))?;
    Ok(())
}

pub fn delete() -> Result<()> {
    if std::env::var("BB_KEYRING_DISABLE").is_ok() {
        return Ok(());
    }

    // A missing entry is not an error for `logout`.
    store::delete(KEYRING_SERVICE, KEYRING_USER);
    store::delete(KEYRING_SERVICE, KEYRING_EMAIL_USER);
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
        // Any future test that exercises the real keyring path (i.e., one that reaches
        // `store` when BB_KEYRING_DISABLE is unset) will panic if it runs after this
        // test. To avoid this:
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

    /// An in-memory keyring, so the non-macOS store is exercised without a
    /// secret-service daemon and without touching the developer's own keyring.
    #[cfg(not(target_os = "macos"))]
    mod memory_keyring {
        use keyring::credential::{Credential, CredentialApi, CredentialBuilderApi};
        use std::collections::HashMap;
        use std::sync::{Arc, Mutex};

        type Items = Arc<Mutex<HashMap<String, Vec<u8>>>>;

        pub struct Builder {
            pub items: Items,
            pub refuse_writes_for: Option<&'static str>,
        }

        struct Item {
            items: Items,
            key: String,
            refuse_writes: bool,
        }

        impl CredentialApi for Item {
            fn set_secret(&self, secret: &[u8]) -> keyring::Result<()> {
                if self.refuse_writes {
                    return Err(keyring::Error::PlatformFailure("locked".into()));
                }
                self.items
                    .lock()
                    .unwrap()
                    .insert(self.key.clone(), secret.to_vec());
                Ok(())
            }

            fn get_secret(&self) -> keyring::Result<Vec<u8>> {
                let items = self.items.lock().unwrap();
                items.get(&self.key).cloned().ok_or(keyring::Error::NoEntry)
            }

            fn delete_credential(&self) -> keyring::Result<()> {
                let removed = self.items.lock().unwrap().remove(&self.key);
                removed.map(drop).ok_or(keyring::Error::NoEntry)
            }

            fn as_any(&self) -> &dyn std::any::Any {
                self
            }
        }

        impl CredentialBuilderApi for Builder {
            fn build(
                &self,
                _target: Option<&str>,
                service: &str,
                user: &str,
            ) -> keyring::Result<Box<Credential>> {
                Ok(Box::new(Item {
                    items: Arc::clone(&self.items),
                    key: format!("{service}/{user}"),
                    refuse_writes: self.refuse_writes_for == Some(user),
                }))
            }

            fn as_any(&self) -> &dyn std::any::Any {
                self
            }
        }

        pub fn install(refuse_writes_for: Option<&'static str>) {
            keyring::set_default_credential_builder(Box::new(Builder {
                items: Items::default(),
                refuse_writes_for,
            }));
        }
    }

    #[cfg(not(target_os = "macos"))]
    fn clear_credential_env() {
        for var in ["BB_EMAIL", "BB_TOKEN", "BB_KEYRING_DISABLE"] {
            std::env::remove_var(var);
        }
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    #[serial]
    fn credentials_round_trip_through_the_keyring() {
        memory_keyring::install(None);
        clear_credential_env();

        assert!(matches!(load(), Err(BbError::Auth)));
        store("dev@example.com", &SecretString::from("s3cr3t")).unwrap();
        let creds = load().unwrap();
        assert_eq!(creds.email, "dev@example.com");
        assert_eq!(creds.token.expose_secret(), "s3cr3t");

        delete().unwrap();
        assert!(matches!(load(), Err(BbError::Auth)));
        delete().unwrap();
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    #[serial]
    fn a_refused_keyring_write_names_what_was_not_stored() {
        clear_credential_env();
        for (account, what) in [(KEYRING_USER, "token"), (KEYRING_EMAIL_USER, "email")] {
            memory_keyring::install(Some(account));
            let err = store("dev@example.com", &SecretString::from("s3cr3t"))
                .unwrap_err()
                .to_string();
            assert!(
                err.contains(&format!("cannot write {what} to keyring")),
                "{err}"
            );
            assert!(err.contains("locked"), "{err}");
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_security_command_never_carries_the_plaintext_secret() {
        let line = store::add_command("bb-cli", "bitbucket-api-token", "ATATT_s3cr3t");
        assert!(!line.contains("ATATT_s3cr3t"), "{line}");
        assert!(line.ends_with(" -X 41544154545f733363723374\n"), "{line}");
    }

    /// Touches the real login keychain, so it runs only on request:
    /// `cargo test --lib credentials -- --ignored`.
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore]
    fn the_macos_keychain_round_trips_through_security() {
        let service = format!("bb-cli-test-{}", std::process::id());
        store::set(&service, "acct", "first=value_1").unwrap();
        store::set(&service, "acct", "second").unwrap();
        assert_eq!(store::get(&service, "acct").as_deref(), Some("second"));
        store::delete(&service, "acct");
        assert_eq!(store::get(&service, "acct"), None);
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
