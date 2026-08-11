use crate::error::{BbError, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

pub const SKILL_NAME: &str = "bitbucket-cloud";

/// The skill text ships *inside* the binary, so every upgrade path — brew,
/// cargo, `bb update` — carries new content as an inherent consequence rather
/// than needing a separate sync. It also means the installed skill can never
/// describe a flag this binary lacks.
pub const SKILL_MD: &str = include_str!("../.agents/skills/bitbucket-cloud/SKILL.md");

pub fn content_hash(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Agent {
    /// `.agents/skills/` — read by Codex, Cursor and OpenCode.
    Agents,
    /// `.claude/skills/` — Claude Code reads only this location.
    Claude,
}

impl Agent {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Agents => "agents",
            Self::Claude => "claude",
        }
    }

    pub fn all() -> [Agent; 2] {
        [Agent::Agents, Agent::Claude]
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub path: PathBuf,
    pub agent: String,
    /// `"file"` or `"symlink"` — a refresh has to rewrite the real file, and an
    /// uninstall has to remove the right kind of thing.
    pub kind: String,
    /// Hash of what bb itself wrote. Comparing it against the file on disk is
    /// how a local edit is detected and protected.
    pub sha256: String,
    pub version: String,
}

pub fn state_path() -> PathBuf {
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
        if !xdg.is_empty() {
            return PathBuf::from(xdg).join("bb").join("skills.json");
        }
    }
    let home = std::env::var_os("HOME").unwrap_or_default();
    PathBuf::from(home)
        .join(".config")
        .join("bb")
        .join("skills.json")
}

/// Entries plus an optional warning. A missing state file simply means nothing
/// is tracked; a corrupt one is reported but treated as empty, so a hand-edited
/// file cannot brick `bb update`.
pub fn load_state() -> (Vec<Entry>, Option<String>) {
    let path = state_path();
    let raw = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return (Vec::new(), None),
        Err(err) => {
            return (
                Vec::new(),
                Some(format!("could not read {}: {err}", path.display())),
            )
        }
    };
    if raw.trim().is_empty() {
        return (Vec::new(), None);
    }
    match serde_json::from_str::<Vec<Entry>>(&raw) {
        Ok(entries) => (entries, None),
        Err(err) => (
            Vec::new(),
            Some(format!("ignoring unreadable {}: {err}", path.display())),
        ),
    }
}

pub fn save_state(entries: &[Entry]) -> Result<()> {
    let path = state_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(BbError::Io)?;
    }
    let json = serde_json::to_string_pretty(entries)?;
    std::fs::write(&path, json).map_err(BbError::Io)?;
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    /// A packaging regression — an added `exclude` entry in Cargo.toml, or a moved
    /// file — must fail the build rather than ship an empty skill.
    #[test]
    fn embedded_skill_is_present_and_has_frontmatter() {
        assert!(!SKILL_MD.trim().is_empty());
        assert!(
            SKILL_MD.starts_with("---"),
            "skill must open with yaml frontmatter"
        );
        assert!(
            SKILL_MD.contains("name: bitbucket-cloud"),
            "frontmatter should name the skill"
        );
    }

    #[test]
    fn content_hash_is_stable_and_distinguishes_content() {
        assert_eq!(content_hash(b"abc"), content_hash(b"abc"));
        assert_ne!(content_hash(b"abc"), content_hash(b"abd"));
        // sha256 hex is 64 chars
        assert_eq!(content_hash(b"abc").len(), 64);
    }

    #[test]
    #[serial_test::serial]
    fn state_path_prefers_xdg_config_home() {
        temp_env(
            &[
                ("XDG_CONFIG_HOME", Some("/tmp/xdg")),
                ("HOME", Some("/tmp/home")),
            ],
            || {
                assert_eq!(
                    state_path(),
                    std::path::Path::new("/tmp/xdg/bb/skills.json")
                );
            },
        );
    }

    #[test]
    #[serial_test::serial]
    fn state_path_falls_back_to_home_config() {
        temp_env(
            &[("XDG_CONFIG_HOME", None), ("HOME", Some("/tmp/home"))],
            || {
                assert_eq!(
                    state_path(),
                    std::path::Path::new("/tmp/home/.config/bb/skills.json")
                );
            },
        );
    }

    #[test]
    #[serial_test::serial]
    fn saved_state_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        temp_env(
            &[
                ("XDG_CONFIG_HOME", Some(dir.path().to_str().unwrap())),
                ("HOME", None),
            ],
            || {
                let entries = vec![Entry {
                    path: std::path::PathBuf::from("/p/.agents/skills/bitbucket-cloud/SKILL.md"),
                    agent: "agents".into(),
                    kind: "file".into(),
                    sha256: content_hash(SKILL_MD.as_bytes()),
                    version: env!("CARGO_PKG_VERSION").into(),
                }];
                save_state(&entries).unwrap();
                let (loaded, warning) = load_state();
                assert!(warning.is_none());
                assert_eq!(loaded.len(), 1);
                assert_eq!(loaded[0].agent, "agents");
                assert_eq!(loaded[0].sha256, entries[0].sha256);
            },
        );
    }

    /// A hand-edited or truncated state file must not brick the command.
    #[test]
    #[serial_test::serial]
    fn corrupt_state_is_tolerated_with_a_warning() {
        let dir = tempfile::tempdir().unwrap();
        temp_env(
            &[
                ("XDG_CONFIG_HOME", Some(dir.path().to_str().unwrap())),
                ("HOME", None),
            ],
            || {
                let p = state_path();
                std::fs::create_dir_all(p.parent().unwrap()).unwrap();
                std::fs::write(&p, "{not json").unwrap();
                let (loaded, warning) = load_state();
                assert!(loaded.is_empty());
                assert!(warning.is_some(), "corrupt state should warn");
            },
        );
    }

    #[test]
    #[serial_test::serial]
    fn missing_state_is_empty_and_silent() {
        let dir = tempfile::tempdir().unwrap();
        temp_env(
            &[
                ("XDG_CONFIG_HOME", Some(dir.path().to_str().unwrap())),
                ("HOME", None),
            ],
            || {
                let (loaded, warning) = load_state();
                assert!(loaded.is_empty());
                assert!(warning.is_none());
            },
        );
    }

    /// Restores saved env vars on drop, so a panic inside `temp_env`'s closure
    /// still puts `HOME`/`XDG_CONFIG_HOME` back rather than leaking a
    /// soon-to-be-dropped tempdir path into whichever `#[serial]` test runs next.
    struct EnvGuard {
        saved: Vec<(String, Option<String>)>,
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (k, v) in &self.saved {
                match v {
                    Some(val) => std::env::set_var(k, val),
                    None => std::env::remove_var(k),
                }
            }
        }
    }

    /// Sets env vars for the closure and restores them afterwards, even if the
    /// closure panics. `None` removes. Tests that call this must be `#[serial]`,
    /// because process env is global.
    fn temp_env(vars: &[(&str, Option<&str>)], f: impl FnOnce()) {
        let saved: Vec<(String, Option<String>)> = vars
            .iter()
            .map(|(k, _)| ((*k).to_string(), std::env::var(k).ok()))
            .collect();
        let _guard = EnvGuard { saved };
        for (k, v) in vars {
            match v {
                Some(val) => std::env::set_var(k, val),
                None => std::env::remove_var(k),
            }
        }
        f();
    }

    #[test]
    #[serial_test::serial]
    fn temp_env_restores_vars_even_if_the_closure_panics() {
        std::env::set_var("XDG_CONFIG_HOME", "/before/panic");
        std::env::remove_var("HOME");

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            temp_env(
                &[
                    ("XDG_CONFIG_HOME", Some("/tmp/during-panic")),
                    ("HOME", Some("/tmp/home")),
                ],
                || panic!("simulated test failure inside temp_env"),
            );
        }));
        assert!(result.is_err(), "closure should have panicked");

        assert_eq!(
            std::env::var("XDG_CONFIG_HOME").ok(),
            Some("/before/panic".to_string()),
            "XDG_CONFIG_HOME must be restored even after a panic"
        );
        assert_eq!(
            std::env::var("HOME").ok(),
            None,
            "HOME must be restored to unset even after a panic"
        );

        std::env::remove_var("XDG_CONFIG_HOME");
    }
}
