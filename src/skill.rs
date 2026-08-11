use crate::error::{BbError, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Installed,
    Refreshed,
    Unchanged,
    SkippedModified,
}

impl Action {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Installed => "installed",
            Self::Refreshed => "refreshed",
            Self::Unchanged => "unchanged",
            Self::SkippedModified => "skipped_modified",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Outcome {
    pub path: PathBuf,
    pub agent: String,
    pub action: Action,
}

/// Where the real file lives for each agent.
pub fn skill_file(root: &Path, agent: Agent) -> PathBuf {
    let base = match agent {
        Agent::Agents => root.join(".agents").join("skills"),
        Agent::Claude => root.join(".claude").join("skills"),
    };
    base.join(SKILL_NAME).join("SKILL.md")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Current,
    Stale,
    Modified,
    Missing,
}

impl State {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Current => "current",
            Self::Stale => "stale",
            Self::Modified => "modified",
            Self::Missing => "missing",
        }
    }
}

#[derive(Debug, Clone)]
pub struct StatusRow {
    pub path: PathBuf,
    pub agent: String,
    pub state: State,
}

fn state_of(entry: &Entry, wanted: &str) -> State {
    match std::fs::read(&entry.path) {
        Err(_) => State::Missing,
        Ok(bytes) => {
            let actual = content_hash(&bytes);
            if actual == wanted {
                State::Current
            } else if actual == entry.sha256 {
                State::Stale
            } else {
                State::Modified
            }
        }
    }
}

pub fn status() -> (Vec<StatusRow>, Option<String>) {
    let (entries, warning) = load_state();
    let wanted = content_hash(SKILL_MD.as_bytes());
    let rows = entries
        .iter()
        .map(|e| StatusRow {
            path: e.path.clone(),
            agent: e.agent.clone(),
            state: state_of(e, &wanted),
        })
        .collect();
    (rows, warning)
}

/// Removes what bb recorded. A customized file is left in place unless `force`,
/// and an untracked file is never touched at all.
pub fn uninstall(root: Option<&Path>, force: bool) -> Result<Vec<(PathBuf, bool)>> {
    let (entries, warning) = load_state();
    if let Some(warning) = warning {
        crate::output::warn(&warning);
    }
    let wanted = content_hash(SKILL_MD.as_bytes());
    let mut results = Vec::new();
    let mut keep = Vec::new();

    for entry in entries {
        let in_scope = root.is_none_or(|r| entry.path.starts_with(r));
        if !in_scope {
            keep.push(entry);
            continue;
        }
        let modified = matches!(state_of(&entry, &wanted), State::Modified);
        if modified && !force {
            results.push((entry.path.clone(), false));
            keep.push(entry);
            continue;
        }
        // A symlinked Claude entry's `path` is `SKILL.md` *inside* the linked
        // directory, so removing it directly would follow the link and delete
        // the `.agents` copy it points at. The thing actually on disk at the
        // Claude location is the symlink one level up — remove that instead,
        // and don't recurse into what it points to.
        let removal_target: &Path = if entry.kind == "symlink" {
            entry.path.parent().unwrap_or(&entry.path)
        } else {
            &entry.path
        };
        let existed = removal_target.exists() || std::fs::symlink_metadata(removal_target).is_ok();
        remove_existing(removal_target)?;
        results.push((entry.path.clone(), existed));
    }

    save_state(&keep)?;
    Ok(results)
}

/// `.cursor/` and `.opencode/` both read `.agents/skills/`, so their presence
/// asks for the `.agents` write rather than a location of their own.
pub fn detect_agents(root: &Path) -> Vec<Agent> {
    let mut found = Vec::new();
    let shares_agents = [".agents", ".cursor", ".opencode"]
        .iter()
        .any(|d| root.join(d).is_dir());
    if shares_agents {
        found.push(Agent::Agents);
    }
    if root.join(".claude").is_dir() {
        found.push(Agent::Claude);
    }
    found
}

fn write_file(path: &Path, contents: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(BbError::Io)?;
    }
    std::fs::write(path, contents).map_err(BbError::Io)?;
    Ok(())
}

/// Removes whatever is at `path` — file, dir, or symlink — without following a
/// symlink into its target. `remove_file` handles symlinks-to-files and plain
/// files; a symlink-to-directory needs `remove_dir_all` refusing to peek inside
/// on most platforms, but to be safe we check `symlink_metadata` first.
fn remove_existing(path: &Path) -> Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) => {
            if meta.file_type().is_symlink() || !meta.is_dir() {
                std::fs::remove_file(path).map_err(BbError::Io)?;
            } else {
                std::fs::remove_dir_all(path).map_err(BbError::Io)?;
            }
            Ok(())
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(BbError::Io(err)),
    }
}

/// Installs the Claude copy as a relative symlink to the `.agents` skill
/// directory when both are present, falling back to a real file otherwise.
/// Returns the `kind` that was actually written ("symlink" or "file").
fn install_claude_dir(root: &Path, agents_installed: bool) -> Result<&'static str> {
    let claude_dir = root.join(".claude").join("skills").join(SKILL_NAME);
    let claude_file = claude_dir.join("SKILL.md");

    if agents_installed {
        #[cfg(unix)]
        {
            if let Some(parent) = claude_dir.parent() {
                std::fs::create_dir_all(parent).map_err(BbError::Io)?;
            }
            remove_existing(&claude_dir)?;
            let target = Path::new("..")
                .join("..")
                .join(".agents")
                .join("skills")
                .join(SKILL_NAME);
            if std::os::unix::fs::symlink(&target, &claude_dir).is_ok() {
                return Ok("symlink");
            }
        }
    }

    remove_existing(&claude_dir)?;
    write_file(&claude_file, SKILL_MD)?;
    Ok("file")
}

pub fn install(root: &Path, agents: &[Agent], force: bool) -> Result<Vec<Outcome>> {
    let (mut state, _warning) = load_state();
    let wanted = content_hash(SKILL_MD.as_bytes());
    let mut outcomes = Vec::new();

    let agents_dir_present = root
        .join(".agents")
        .join("skills")
        .join(SKILL_NAME)
        .join("SKILL.md")
        .exists()
        || agents.contains(&Agent::Agents);

    for agent in agents {
        let path = skill_file(root, *agent);
        let recorded = state.iter().find(|e| e.path == path).cloned();
        let on_disk = std::fs::read(&path).ok();

        let action = match (&on_disk, &recorded) {
            (None, _) => Action::Installed,
            (Some(bytes), _) if content_hash(bytes) == wanted => Action::Unchanged,
            // We wrote it and the binary now carries newer text.
            (Some(bytes), Some(entry)) if content_hash(bytes) == entry.sha256 => Action::Refreshed,
            // Either untracked or edited since we wrote it — someone's own work.
            (Some(_), _) if force => Action::Refreshed,
            (Some(_), _) => Action::SkippedModified,
        };

        let mut kind = "file".to_string();
        if action != Action::SkippedModified && action != Action::Unchanged {
            if *agent == Agent::Claude {
                kind = install_claude_dir(root, agents_dir_present)?.to_string();
            } else {
                write_file(&path, SKILL_MD)?;
            }
        } else if let Some(entry) = &recorded {
            kind = entry.kind.clone();
        }

        if action != Action::SkippedModified {
            state.retain(|e| e.path != path);
            state.push(Entry {
                path: path.clone(),
                agent: agent.as_str().to_string(),
                kind,
                sha256: wanted.clone(),
                version: env!("CARGO_PKG_VERSION").to_string(),
            });
        }

        outcomes.push(Outcome {
            path,
            agent: agent.as_str().to_string(),
            action,
        });
    }

    save_state(&state)?;
    Ok(outcomes)
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

    /// Drives `Stale` (and `Current`) through `status()` end to end, not just
    /// through `install()`'s refresh path. A tracked entry whose sha256 matches
    /// what's on disk, but not what the binary ships now, is stale; a tracked
    /// entry whose sha256 matches the binary's current text is current.
    #[test]
    #[serial_test::serial]
    fn status_reports_stale_when_the_binary_shipped_newer_text() {
        let dir = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        temp_env(
            &[
                ("XDG_CONFIG_HOME", Some(dir.path().to_str().unwrap())),
                ("HOME", None),
            ],
            || {
                let path = skill_file(root.path(), Agent::Agents);
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                let old = "---\nname: bitbucket-cloud\n---\nold text\n";
                std::fs::write(&path, old).unwrap();
                save_state(&[Entry {
                    path: path.clone(),
                    agent: "agents".into(),
                    kind: "file".into(),
                    sha256: content_hash(old.as_bytes()),
                    version: "0.0.1".into(),
                }])
                .unwrap();

                let (rows, warning) = status();
                assert!(warning.is_none());
                assert_eq!(rows.len(), 1);
                assert_eq!(
                    rows[0].state,
                    State::Stale,
                    "on-disk text matches the recorded sha256, just not the binary's current text"
                );

                // Same entry, but now the file holds exactly what the binary ships:
                // that must read as Current, not Stale.
                std::fs::write(&path, SKILL_MD).unwrap();
                save_state(&[Entry {
                    path: path.clone(),
                    agent: "agents".into(),
                    kind: "file".into(),
                    sha256: content_hash(SKILL_MD.as_bytes()),
                    version: env!("CARGO_PKG_VERSION").into(),
                }])
                .unwrap();
                let (rows, _) = status();
                assert_eq!(rows[0].state, State::Current);
            },
        );
    }

    /// Uninstalling a symlinked Claude entry must remove the link itself, not
    /// follow it into the `.agents` copy it points at. Scopes the uninstall to
    /// just the Claude subtree so the `.agents` entry is never itself in scope
    /// for removal — the only way to prove the target survives *because* the
    /// link wasn't followed, rather than because it was also being deleted on
    /// its own account.
    #[test]
    #[serial_test::serial]
    fn uninstall_removes_the_claude_link_without_deleting_its_target() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = tempfile::tempdir().unwrap();
        temp_env(
            &[
                ("XDG_CONFIG_HOME", Some(cfg.path().to_str().unwrap())),
                ("HOME", None),
            ],
            || {
                install(dir.path(), &[Agent::Agents, Agent::Claude], false).unwrap();
                let agents_path = skill_file(dir.path(), Agent::Agents);
                let claude_root = dir.path().join(".claude");
                assert!(agents_path.is_file(), "sanity: agents copy installed");

                let results = uninstall(Some(&claude_root), false).unwrap();
                assert_eq!(results.len(), 1, "only the claude entry was in scope");
                assert!(results[0].1, "the claude entry should report removed");

                let claude_dir = dir.path().join(".claude/skills/bitbucket-cloud");
                assert!(
                    !claude_dir.exists(),
                    "the claude link (or fallback file) should be gone"
                );
                assert!(
                    agents_path.is_file(),
                    "removing the claude link must not delete the agents copy it points at"
                );

                let (remaining, _) = load_state();
                assert_eq!(remaining.len(), 1, "the agents entry stays tracked");
                assert_eq!(remaining[0].agent, "agents");
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

    #[test]
    #[serial_test::serial]
    fn install_writes_the_embedded_content() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = tempfile::tempdir().unwrap();
        temp_env(
            &[
                ("XDG_CONFIG_HOME", Some(cfg.path().to_str().unwrap())),
                ("HOME", None),
            ],
            || {
                let outcomes = install(dir.path(), &[Agent::Agents], false).unwrap();
                assert_eq!(outcomes.len(), 1);
                assert!(matches!(outcomes[0].action, Action::Installed));

                let written =
                    std::fs::read_to_string(skill_file(dir.path(), Agent::Agents)).unwrap();
                assert_eq!(
                    written, SKILL_MD,
                    "installed content must equal the embedded skill"
                );

                let (state, _) = load_state();
                assert_eq!(state.len(), 1);
                assert_eq!(state[0].sha256, content_hash(SKILL_MD.as_bytes()));
            },
        );
    }

    #[test]
    #[serial_test::serial]
    fn a_second_install_reports_unchanged_and_rewrites_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = tempfile::tempdir().unwrap();
        temp_env(
            &[
                ("XDG_CONFIG_HOME", Some(cfg.path().to_str().unwrap())),
                ("HOME", None),
            ],
            || {
                install(dir.path(), &[Agent::Agents], false).unwrap();
                let outcomes = install(dir.path(), &[Agent::Agents], false).unwrap();
                assert!(
                    matches!(outcomes[0].action, Action::Unchanged),
                    "{:?}",
                    outcomes[0].action
                );
            },
        );
    }

    /// A local edit is somebody's deliberate customization. It must survive.
    #[test]
    #[serial_test::serial]
    fn a_modified_file_is_refused_and_left_byte_identical() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = tempfile::tempdir().unwrap();
        temp_env(
            &[
                ("XDG_CONFIG_HOME", Some(cfg.path().to_str().unwrap())),
                ("HOME", None),
            ],
            || {
                install(dir.path(), &[Agent::Agents], false).unwrap();
                let path = skill_file(dir.path(), Agent::Agents);
                std::fs::write(&path, "# my own notes\n").unwrap();

                let outcomes = install(dir.path(), &[Agent::Agents], false).unwrap();
                assert!(matches!(outcomes[0].action, Action::SkippedModified));
                assert_eq!(std::fs::read_to_string(&path).unwrap(), "# my own notes\n");
            },
        );
    }

    #[test]
    #[serial_test::serial]
    fn force_overwrites_a_modified_file_and_updates_the_hash() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = tempfile::tempdir().unwrap();
        temp_env(
            &[
                ("XDG_CONFIG_HOME", Some(cfg.path().to_str().unwrap())),
                ("HOME", None),
            ],
            || {
                install(dir.path(), &[Agent::Agents], false).unwrap();
                let path = skill_file(dir.path(), Agent::Agents);
                std::fs::write(&path, "# my own notes\n").unwrap();

                let outcomes = install(dir.path(), &[Agent::Agents], true).unwrap();
                assert!(matches!(
                    outcomes[0].action,
                    Action::Refreshed | Action::Installed
                ));
                assert_eq!(std::fs::read_to_string(&path).unwrap(), SKILL_MD);
                let (state, _) = load_state();
                assert_eq!(state[0].sha256, content_hash(SKILL_MD.as_bytes()));
            },
        );
    }

    /// Stale means "we wrote it, and the binary has newer text now". It refreshes
    /// without asking, because nobody customized it.
    #[test]
    #[serial_test::serial]
    fn a_stale_file_is_refreshed_silently() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = tempfile::tempdir().unwrap();
        temp_env(
            &[
                ("XDG_CONFIG_HOME", Some(cfg.path().to_str().unwrap())),
                ("HOME", None),
            ],
            || {
                let path = skill_file(dir.path(), Agent::Agents);
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                let old = "---\nname: bitbucket-cloud\n---\nold text\n";
                std::fs::write(&path, old).unwrap();
                save_state(&[Entry {
                    path: path.clone(),
                    agent: "agents".into(),
                    kind: "file".into(),
                    sha256: content_hash(old.as_bytes()),
                    version: "0.0.1".into(),
                }])
                .unwrap();

                let outcomes = install(dir.path(), &[Agent::Agents], false).unwrap();
                assert!(
                    matches!(outcomes[0].action, Action::Refreshed),
                    "{:?}",
                    outcomes[0].action
                );
                assert_eq!(std::fs::read_to_string(&path).unwrap(), SKILL_MD);
            },
        );
    }

    #[test]
    #[serial_test::serial]
    fn claude_install_links_to_the_agents_copy() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = tempfile::tempdir().unwrap();
        temp_env(
            &[
                ("XDG_CONFIG_HOME", Some(cfg.path().to_str().unwrap())),
                ("HOME", None),
            ],
            || {
                install(dir.path(), &[Agent::Agents, Agent::Claude], false).unwrap();
                let claude = dir.path().join(".claude/skills").join(SKILL_NAME);
                // Either a symlink resolving to the .agents copy, or a real file with
                // the same content when the platform refused a symlink.
                let content = std::fs::read_to_string(claude.join("SKILL.md"))
                    .or_else(|_| std::fs::read_to_string(&claude))
                    .unwrap();
                assert_eq!(content, SKILL_MD);
            },
        );
    }

    #[test]
    fn detect_finds_each_agent_directory() {
        let dir = tempfile::tempdir().unwrap();
        assert!(
            detect_agents(dir.path()).is_empty(),
            "nothing present means nothing detected"
        );

        std::fs::create_dir_all(dir.path().join(".cursor")).unwrap();
        assert_eq!(
            detect_agents(dir.path()),
            vec![Agent::Agents],
            "cursor reads .agents"
        );

        std::fs::create_dir_all(dir.path().join(".claude")).unwrap();
        let found = detect_agents(dir.path());
        assert!(found.contains(&Agent::Agents) && found.contains(&Agent::Claude));
    }
}
