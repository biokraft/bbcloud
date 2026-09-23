//! The passive "a newer bb exists" notice.
//!
//! `bb update` answers the question only when someone thinks to ask it, and
//! most people never do — so this asks on their behalf, once a day, ahead of
//! whatever command they actually ran.
//!
//! Four properties, each with a test, because this runs before *every*
//! command and must never be the reason one of them fails or changes shape:
//!
//! - the notice goes to **stderr**, in human and `--json` mode alike, so the
//!   `--json` stdout contract stays intact and an agent reading stderr still
//!   learns about the upgrade;
//! - the network is touched at most once per [`CHECK_TTL`]; every other
//!   invocation reads one small file;
//! - every failure — offline, rate limited, unwritable config dir, corrupt
//!   cache — is swallowed silently, since the user asked for something else;
//! - `BB_NO_UPDATE_CHECK=1` turns the whole thing off.

use crate::commands::update::{self, is_newer};
use crate::output::{self, Format};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// How long a recorded answer is trusted. A day is short enough that an
/// upgrade is noticed promptly and long enough that the check is invisible in
/// normal use.
pub const CHECK_TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// Hard ceiling on how long a user's command may be delayed by the check.
/// They did not ask for this request, so it gets a fraction of the budget
/// `bb update` gives its own.
const CHECK_TIMEOUT: Duration = Duration::from_secs(2);

/// What the last check found. `latest` is stored as the raw tag (`v0.19.4`)
/// because that is what the release API returns and [`is_newer`] tolerates
/// the `v`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cache {
    /// Unix seconds at which the release API was last asked.
    pub checked_at: u64,
    /// The newest tag it reported.
    pub latest: String,
}

pub fn cache_path() -> PathBuf {
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
        if !xdg.is_empty() {
            return PathBuf::from(xdg).join("bb").join("update-check.json");
        }
    }
    let home = std::env::var_os("HOME").unwrap_or_default();
    PathBuf::from(home)
        .join(".config")
        .join("bb")
        .join("update-check.json")
}

/// A missing or unreadable cache is indistinguishable from never having
/// checked, which is exactly the right reading: check again.
pub fn load_cache() -> Option<Cache> {
    let raw = std::fs::read_to_string(cache_path()).ok()?;
    serde_json::from_str(&raw).ok()
}

/// Written the same way `skill::save_state` writes: temp file plus `rename`,
/// so two `bb` processes racing cannot leave a truncated file that then
/// silently disables the check.
pub fn save_cache(cache: &Cache) -> std::io::Result<()> {
    let path = cache_path();
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    std::fs::create_dir_all(parent)?;
    let json = serde_json::to_string(cache)
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?;
    let tmp = parent.join(format!(
        ".update-check.json.tmp.{}.{}",
        std::process::id(),
        now_secs()
    ));
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, &path)
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// True when the cache is missing, older than `ttl`, or stamped in the
/// future. A future stamp means a clock moved backwards; treating it as fresh
/// would freeze the check until the clock caught up.
pub fn is_stale(cache: Option<&Cache>, now: u64, ttl: Duration) -> bool {
    match cache {
        None => true,
        Some(cache) => cache.checked_at > now || now - cache.checked_at >= ttl.as_secs(),
    }
}

/// The line shown when `latest` is ahead of `current`, and `None` otherwise.
///
/// It names the version, the running version and the one command that
/// upgrades *this* install, because a notice that says only "update
/// available" makes the reader go looking for how.
pub fn notice(latest: &str, current: &str, hint: &str) -> Option<String> {
    if !is_newer(latest, current) {
        return None;
    }
    let version = latest.trim().trim_start_matches('v');
    Some(format!(
        "bb {version} is available (you have {current}) — upgrade with: {hint}"
    ))
}

fn hint_for_this_install() -> &'static str {
    match std::env::current_exe() {
        Ok(exe) => update::upgrade_hint(update::classify_install(&exe)),
        // Without a path there is nothing to classify. `bb update` works for
        // a standalone install and tells the other two what to run instead,
        // so it is the safe answer rather than a guess at brew or cargo.
        Err(_) => "bb update",
    }
}

fn client() -> Option<reqwest::Client> {
    reqwest::Client::builder()
        .connect_timeout(CHECK_TIMEOUT)
        .timeout(CHECK_TIMEOUT)
        .user_agent(concat!("bbcloud/", env!("CARGO_PKG_VERSION")))
        .build()
        .ok()
}

/// The pre-command hook. Prints at most one line, to stderr, and returns
/// nothing to fail on.
///
/// `format` is taken only so a future format can opt out; the notice is
/// printed in both current formats, since stderr is outside the `--json`
/// stdout contract and an agent needs the notice as much as a human does.
pub async fn maybe_notify(format: Format, base_url: &str) {
    let _ = format;
    if std::env::var_os("BB_NO_UPDATE_CHECK").is_some() {
        return;
    }
    let current = env!("CARGO_PKG_VERSION");
    let mut cache = load_cache();

    if is_stale(cache.as_ref(), now_secs(), CHECK_TTL) {
        if let Some(http) = client() {
            if let Ok(latest) = update::latest_tag(&http, base_url).await {
                let fresh = Cache {
                    checked_at: now_secs(),
                    latest,
                };
                // A cache that cannot be written costs an extra request a day,
                // which is not worth a line of noise on someone's command.
                let _ = save_cache(&fresh);
                cache = Some(fresh);
            }
        }
    }

    if let Some(line) = cache
        .as_ref()
        .and_then(|c| notice(&c.latest, current, hint_for_this_install()))
    {
        output::warn(&line);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn notice_names_version_and_command() {
        let line = notice("v0.20.0", "0.19.4", "bb update").unwrap();
        assert!(line.contains("bb 0.20.0 is available"), "{line}");
        assert!(line.contains("you have 0.19.4"), "{line}");
        assert!(line.ends_with("bb update"), "{line}");
    }

    #[test]
    fn no_notice_when_current_or_ahead() {
        assert!(notice("v0.19.4", "0.19.4", "bb update").is_none());
        assert!(notice("v0.19.3", "0.19.4", "bb update").is_none());
    }

    #[test]
    fn unparseable_tag_is_never_a_notice() {
        assert!(notice("nightly", "0.19.4", "bb update").is_none());
        assert!(notice("", "0.19.4", "bb update").is_none());
    }

    #[test]
    fn missing_cache_is_stale() {
        assert!(is_stale(None, 1_000_000, CHECK_TTL));
    }

    #[test]
    fn fresh_cache_is_not_stale() {
        let cache = Cache {
            checked_at: 1_000_000,
            latest: "v0.19.4".to_string(),
        };
        assert!(!is_stale(Some(&cache), 1_000_000 + 60, CHECK_TTL));
    }

    #[test]
    fn cache_older_than_ttl_is_stale() {
        let cache = Cache {
            checked_at: 1_000_000,
            latest: "v0.19.4".to_string(),
        };
        assert!(is_stale(
            Some(&cache),
            1_000_000 + CHECK_TTL.as_secs(),
            CHECK_TTL
        ));
    }

    #[test]
    fn cache_stamped_in_the_future_is_stale() {
        let cache = Cache {
            checked_at: 2_000_000,
            latest: "v0.19.4".to_string(),
        };
        assert!(is_stale(Some(&cache), 1_000_000, CHECK_TTL));
    }
}
