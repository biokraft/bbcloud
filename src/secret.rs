pub use secrecy::{ExposeSecret, SecretString};

/// Renders a secret for human display. Never returns more than the last four
/// characters, and returns nothing identifying for short values.
pub fn redact(value: &str) -> String {
    let len = value.chars().count();
    if len < 8 {
        return "****".to_string();
    }
    let tail: String = value.chars().skip(len - 4).collect();
    format!("****{tail}")
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn redacts_short_values_entirely() {
        assert_eq!(redact("abc"), "****");
        assert_eq!(redact(""), "****");
    }

    #[test]
    fn redacts_all_but_last_four() {
        assert_eq!(redact("ATATT3xFfGF0abcd"), "****abcd");
    }

    #[test]
    fn redaction_never_contains_the_secret_prefix() {
        let secret = "ATATT3xFfGF0_super_secret_value";
        let shown = redact(secret);
        assert!(!shown.contains("ATATT"), "prefix leaked: {shown}");
        assert!(!shown.contains("super_secret"), "body leaked: {shown}");
    }
}
