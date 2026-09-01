use crate::error::{BbError, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// One embedded skill. The text ships *inside* the binary, so every upgrade
/// path — brew, cargo, `bb update` — carries new content as an inherent
/// consequence rather than needing a separate sync. It also means an installed
/// skill can never describe a flag this binary lacks.
pub struct Skill {
    pub name: &'static str,
    /// One line, shown as this skill's row in the `bb skill install` prompt.
    /// Kept short enough to render on a narrow terminal.
    pub summary: &'static str,
    pub content: &'static str,
}

pub const SKILLS: [Skill; 4] = [
    Skill {
        name: "bitbucket-cloud",
        summary: "read, review and comment on Bitbucket Cloud pull requests",
        content: include_str!("../.agents/skills/bitbucket-cloud/SKILL.md"),
    },
    Skill {
        name: "bbc-daily-brief",
        summary: "a ranked morning brief of the pull requests waiting on you",
        content: include_str!("../.agents/skills/bbc-daily-brief/SKILL.md"),
    },
    Skill {
        name: "bbc-open-pr",
        summary: "open a pull request: reviewer suggestions from git history",
        content: include_str!("../.agents/skills/bbc-open-pr/SKILL.md"),
    },
    Skill {
        name: "bbc-report-bug",
        summary: "file a bb bug upstream: reproduce, redact, ask, then gh",
        content: include_str!("../.agents/skills/bbc-report-bug/SKILL.md"),
    },
];

pub fn skill_by_name(name: &str) -> Option<&'static Skill> {
    SKILLS.iter().find(|s| s.name == name)
}

/// State files written before the second skill existed carry no `skill` field.
/// They can only have described the first one.
fn default_skill_name() -> String {
    "bitbucket-cloud".to_string()
}

pub fn content_hash(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

/// True when any tracked entry was written by a different build than the one
/// running now. This is the whole auto-refresh trigger: a string compare over a
/// handful of entries, so the common case — everything current — costs nothing
/// beyond reading the state file.
pub fn tracked_version_differs(entries: &[Entry]) -> bool {
    entries
        .iter()
        .any(|e| e.version != env!("CARGO_PKG_VERSION"))
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
    #[serde(default = "default_skill_name")]
    pub skill: String,
    /// Whether `bb` is what put this file on disk. A content hash cannot answer
    /// this — it proves the bytes match, never who wrote them — so it is
    /// recorded at write time and carried forward. `uninstall` deletes only
    /// what this marks as ours.
    ///
    /// State files written before this field existed carry no value. They
    /// default to `true`: those entries were overwhelmingly bb's own writes,
    /// and defaulting to `false` would strand every skill installed before the
    /// upgrade as un-removable.
    #[serde(default = "default_created")]
    pub created: bool,
}

fn default_created() -> bool {
    true
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

/// Writes via a temp file in the same directory plus `rename`, so two `bb`
/// processes racing right after an upgrade cannot interleave and leave a
/// truncated `skills.json` — `fs::write` truncates first, and `rename` on the
/// same filesystem is atomic where plain writes are not.
pub fn save_state(entries: &[Entry]) -> Result<()> {
    let path = state_path();
    let parent = path.parent().ok_or_else(|| {
        BbError::Config(format!(
            "state path {} has no parent directory",
            path.display()
        ))
    })?;
    std::fs::create_dir_all(parent).map_err(BbError::Io)?;
    let json = serde_json::to_string_pretty(entries)?;
    let tmp = parent.join(format!(
        ".skills.json.tmp.{}.{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::write(&tmp, json).map_err(BbError::Io)?;
    std::fs::rename(&tmp, &path).map_err(BbError::Io)?;
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Installed,
    Refreshed,
    Unchanged,
    SkippedModified,
    /// The entry named a path whose directory tree no longer exists, so it was
    /// dropped from the state file rather than recreated.
    Pruned,
    /// A write that should have brought this entry current failed (EROFS,
    /// EACCES, ...). The entry stays tracked with its old version and hash so
    /// it is retried on the next invocation, rather than aborting every other
    /// entry's refresh in the same batch.
    Failed,
}

impl Action {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Installed => "installed",
            Self::Refreshed => "refreshed",
            Self::Unchanged => "unchanged",
            Self::SkippedModified => "skipped_modified",
            Self::Pruned => "pruned",
            Self::Failed => "failed",
        }
    }
}

/// Whether `refresh_tracked` should recreate an entry whose file has been
/// deleted. Explicit `bb skill install`/`bb update` pass `Restore`, because a
/// human asked for it there; the auto-refresh that runs before every command
/// passes `Preserve`, because a deliberately deleted skill file must not be
/// silently written back into a user's working tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MissingPolicy {
    Restore,
    Preserve,
}

#[derive(Debug, Clone)]
pub struct Outcome {
    pub path: PathBuf,
    pub agent: String,
    pub skill: String,
    pub action: Action,
}

/// Where the real file lives for each agent.
pub fn skill_file(root: &Path, agent: Agent, skill: &Skill) -> PathBuf {
    let base = match agent {
        Agent::Agents => root.join(".agents").join("skills"),
        Agent::Claude => root.join(".claude").join("skills"),
    };
    base.join(skill.name).join("SKILL.md")
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
    pub skill: String,
    pub state: State,
}

/// Refuses any path that does not end in `.agents/skills/<name>/SKILL.md`
/// or `.claude/skills/<name>/SKILL.md`, where `<name>` is a known skill. Every removal or write driven by
/// a state entry must go through this first: the state file is user-editable
/// (by hand or by a bad merge), and nothing it names should let `bb` touch an
/// arbitrary path on disk. Deliberately checks shape, not existence or type —
/// `state_of` already treats a directory as `Missing` (it can't be read as a
/// file), and that used to be enough to make the `Missing` repair branches
/// reach a `remove_dir_all`/`write_file` on whatever the state file named.
fn is_shaped_like_a_skill_path(path: &Path) -> bool {
    let mut components: Vec<_> = path.components().collect();
    let Some(file) = components.pop() else {
        return false;
    };
    if file.as_os_str() != "SKILL.md" {
        return false;
    }
    let Some(skill_dir) = components.pop() else {
        return false;
    };
    if skill_by_name(&skill_dir.as_os_str().to_string_lossy()).is_none() {
        return false;
    }
    let Some(skills_dir) = components.pop() else {
        return false;
    };
    if skills_dir.as_os_str() != "skills" {
        return false;
    }
    matches!(
        components.pop().map(|c| c.as_os_str().to_owned()),
        Some(agents_dir) if agents_dir == ".agents" || agents_dir == ".claude"
    )
}

/// True only when `ancestor` is *definitely* gone — a stat that returns
/// `ENOENT`. `Path::exists()` also reads false on `EACCES` for a parent
/// component, or on a path under an unmounted network/removable volume, and
/// either of those must not be treated as "the tree was deleted": that would
/// prune a still-real entry that will come back once the permission or the
/// mount is restored, leaving the file on disk but untracked forever after.
fn ancestor_is_definitely_gone(ancestor: &Path) -> bool {
    matches!(
        std::fs::symlink_metadata(ancestor),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound
    )
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
    let rows = entries
        .iter()
        .map(|e| {
            // An unknown skill name (a state file written by a newer `bb`, or
            // hand-edited) has no wanted hash to compare against. It must not
            // be able to reach `Stale` — that state promises "the binary has
            // newer text, a refresh will fix it", which is untrue here, since
            // nothing in this binary knows what this entry's content should
            // be. `Modified` correctly refuses to touch it.
            let state = match skill_by_name(&e.skill) {
                Some(skill) => state_of(e, &content_hash(skill.content.as_bytes())),
                None => State::Modified,
            };
            StatusRow {
                path: e.path.clone(),
                agent: e.agent.clone(),
                skill: e.skill.clone(),
                state,
            }
        })
        .collect();
    (rows, warning)
}

/// Distinguishes *why* an entry did not end up removed, so the caller can be
/// honest about it instead of collapsing "refused because modified" and
/// "wasn't there to begin with" into the same boolean.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemovalOutcome {
    Removed,
    RefusedModified,
    /// Tracked, but `bb` never wrote it — a vendored or hand-placed copy that
    /// merely matched the embedded text. Left on disk; the entry is dropped.
    RefusedNotWritten,
    RefusedUnsafePath,
    Absent,
}

impl RemovalOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Removed => "removed",
            Self::RefusedModified => "refused_modified",
            Self::RefusedNotWritten => "refused_not_written",
            Self::RefusedUnsafePath => "refused_unsafe_path",
            Self::Absent => "absent",
        }
    }
}

/// True when the directory *containing* `path` is itself a symlink — the
/// shape a Claude entry takes when `install_claude_dir` linked it. Checked
/// against disk rather than trusted from the recorded `kind`, because `kind`
/// can be wrong: a pre-existing hand-made symlink that `install` finds
/// `Unchanged` or `SkippedModified` (no prior state entry to inherit from)
/// used to default to `kind: "file"`, which then bypassed this exact guard.
fn parent_is_symlink(path: &Path) -> bool {
    path.parent()
        .and_then(|p| std::fs::symlink_metadata(p).ok())
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
}

/// Removes what bb recorded. A customized file is left in place unless `force`,
/// and an untracked file is never touched at all.
pub fn uninstall(
    root: Option<&Path>,
    skills: &[&'static Skill],
    force: bool,
) -> Result<Vec<(PathBuf, String, RemovalOutcome)>> {
    let (entries, warning) = load_state();
    if let Some(warning) = warning {
        crate::output::warn(&warning);
    }
    let in_scope_skill_names: Vec<&str> = skills.iter().map(|s| s.name).collect();
    let mut results = Vec::new();
    let mut keep = Vec::new();

    for entry in entries {
        let in_scope = root.is_none_or(|r| entry.path.starts_with(r))
            && in_scope_skill_names.contains(&entry.skill.as_str());
        if !in_scope {
            keep.push(entry);
            continue;
        }
        if !is_shaped_like_a_skill_path(&entry.path) {
            crate::output::warn(&format!(
                "refusing to touch {} — does not look like a skill path bb would have written",
                entry.path.display()
            ));
            results.push((
                entry.path.clone(),
                entry.skill.clone(),
                RemovalOutcome::RefusedUnsafePath,
            ));
            keep.push(entry);
            continue;
        }
        let wanted = skill_by_name(&entry.skill)
            .map(|s| content_hash(s.content.as_bytes()))
            .unwrap_or_default();
        // A symlinked Claude entry's `path` is `SKILL.md` *inside* the linked
        // directory, so removing it directly would follow the link and delete
        // the `.agents` copy it points at. The thing actually on disk at the
        // Claude location is the symlink one level up — remove that instead,
        // and don't recurse into what it points to. Trusts disk over the
        // recorded `kind`: a hand-made symlink that predates any bb-recorded
        // `kind` must still be removed as a link, not followed.
        let is_symlinked_dir = entry.kind == "symlink" || parent_is_symlink(&entry.path);

        // Never delete a *file* bb did not write. `--force` still overrides, so
        // someone who vendored a copy and explicitly asks for it gone gets
        // that; the entry is dropped either way, because "uninstall" means stop
        // managing it and a refusal that stays tracked is re-reported forever.
        //
        // A symlink is exempt: the guard exists to protect content bb did not
        // author, and a link holds none. Removing a hand-made one — the shape
        // older docs told users to create by hand — loses nothing, and the
        // `.agents` file it points at is protected by this same rule under its
        // own entry.
        if !entry.created && !is_symlinked_dir && !force {
            results.push((
                entry.path.clone(),
                entry.skill.clone(),
                RemovalOutcome::RefusedNotWritten,
            ));
            continue;
        }
        let modified = matches!(state_of(&entry, &wanted), State::Modified);
        if modified && !force {
            results.push((
                entry.path.clone(),
                entry.skill.clone(),
                RemovalOutcome::RefusedModified,
            ));
            keep.push(entry);
            continue;
        }
        let removal_target: &Path = if is_symlinked_dir {
            entry.path.parent().unwrap_or(&entry.path)
        } else {
            &entry.path
        };
        let existed = removal_target.exists() || std::fs::symlink_metadata(removal_target).is_ok();
        remove_existing(removal_target)?;
        // A `kind: "file"` Claude fallback can leave an empty
        // `.claude/skills/<skill name>/` directory behind once `SKILL.md`
        // inside it is gone. Clean up that one directory — never a parent,
        // and never one that still has something in it.
        if !is_symlinked_dir {
            if let Some(dir) = entry.path.parent() {
                let is_empty = std::fs::read_dir(dir)
                    .map(|mut i| i.next().is_none())
                    .unwrap_or(false);
                if is_empty {
                    let _ = std::fs::remove_dir(dir);
                }
            }
        }
        let outcome = if existed {
            RemovalOutcome::Removed
        } else {
            RemovalOutcome::Absent
        };
        results.push((entry.path.clone(), entry.skill.clone(), outcome));
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
fn install_claude_dir(root: &Path, agents_installed: bool, skill: &Skill) -> Result<&'static str> {
    let claude_dir = root.join(".claude").join("skills").join(skill.name);
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
                .join(skill.name);
            if std::os::unix::fs::symlink(&target, &claude_dir).is_ok() {
                return Ok("symlink");
            }
        }
    }

    remove_existing(&claude_dir)?;
    write_file(&claude_file, skill.content)?;
    Ok("file")
}

pub fn install(
    root: &Path,
    agents: &[Agent],
    skills: &[&'static Skill],
    force: bool,
) -> Result<Vec<Outcome>> {
    let (mut state, _warning) = load_state();
    let mut outcomes = Vec::new();

    for skill in skills {
        let wanted = content_hash(skill.content.as_bytes());

        let agents_dir_present = root
            .join(".agents")
            .join("skills")
            .join(skill.name)
            .join("SKILL.md")
            .exists()
            || agents.contains(&Agent::Agents);

        for agent in agents {
            let path = skill_file(root, *agent, skill);
            let recorded = state.iter().find(|e| e.path == path).cloned();
            let on_disk = std::fs::read(&path).ok();

            let action = match (&on_disk, &recorded) {
                (None, _) => Action::Installed,
                (Some(bytes), _) if content_hash(bytes) == wanted => Action::Unchanged,
                // We wrote it and the binary now carries newer text.
                (Some(bytes), Some(entry)) if content_hash(bytes) == entry.sha256 => {
                    Action::Refreshed
                }
                // Either untracked or edited since we wrote it — someone's own work.
                (Some(_), _) if force => Action::Refreshed,
                (Some(_), _) => Action::SkippedModified,
            };

            // Did *this* call put the file there? `Unchanged` means we wrote
            // nothing, so ownership can only be inherited from an existing
            // entry — and when there is none, the file was already on disk and
            // is somebody else's.
            let created = match action {
                Action::Unchanged => recorded.as_ref().map(|e| e.created).unwrap_or(false),
                _ => true,
            };

            let mut kind = "file".to_string();
            if action != Action::SkippedModified && action != Action::Unchanged {
                if *agent == Agent::Claude {
                    kind = install_claude_dir(root, agents_dir_present, skill)?.to_string();
                } else {
                    write_file(&path, skill.content)?;
                }
            } else if let Some(entry) = &recorded {
                kind = entry.kind.clone();
            } else if parent_is_symlink(&path) {
                // No prior state entry to inherit `kind` from — e.g. a hand-made
                // Claude symlink, exactly what older docs told users to create
                // themselves — so it must be read off disk rather than defaulted
                // to `"file"`, or a later uninstall would bypass the symlink
                // guard and follow the link into whatever it points at.
                kind = "symlink".to_string();
            }

            if action != Action::SkippedModified {
                state.retain(|e| e.path != path);
                state.push(Entry {
                    path: path.clone(),
                    agent: agent.as_str().to_string(),
                    kind,
                    sha256: wanted.clone(),
                    version: env!("CARGO_PKG_VERSION").to_string(),
                    skill: skill.name.to_string(),
                    created,
                });
            }

            outcomes.push(Outcome {
                path,
                agent: agent.as_str().to_string(),
                skill: skill.name.to_string(),
                action,
            });
        }
    }

    save_state(&state)?;
    Ok(outcomes)
}

/// A tracked Claude entry's recorded `path` is `SKILL.md` *inside* the linked
/// directory (see `uninstall`'s comment on the same shape), so the project
/// root sits four components above it: `SKILL.md`, `<skill name>`, `skills`,
/// `.claude`.
fn claude_root_from_entry_path(path: &Path) -> Result<&Path> {
    path.parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .and_then(Path::parent)
        .ok_or_else(|| {
            BbError::Config(format!(
                "cannot determine the project root from {}",
                path.display()
            ))
        })
}

/// Recreates a Claude entry whose recorded `kind` is `"symlink"` but whose
/// link (or fallback file) is missing from disk. Delegates to
/// `install_claude_dir` so the same relative-symlink-with-fallback logic that
/// `install` uses is not duplicated here, and returns the `kind` that was
/// actually written so the caller can keep the recorded state honest even
/// when the platform refuses a symlink and falls back to a real file.
fn restore_claude_link(entry_path: &Path, skill: &Skill) -> Result<String> {
    if !is_shaped_like_a_skill_path(entry_path) {
        return Err(BbError::Config(format!(
            "refusing to touch {} — does not look like a skill path bb would have written",
            entry_path.display()
        )));
    }
    let root = claude_root_from_entry_path(entry_path)?;
    let agents_installed = root
        .join(".agents")
        .join("skills")
        .join(skill.name)
        .join("SKILL.md")
        .exists();
    Ok(install_claude_dir(root, agents_installed, skill)?.to_string())
}

/// Refreshes every tracked entry against the currently-running binary's
/// embedded text. Driven by the recorded entries rather than a root and an
/// agent list, so a single call spans every project the user has installed
/// into. Uses the same drift rules as `install`, via `state_of`: `Stale`
/// rewrites the file and updates the recorded hash, `Modified` is left
/// byte-identical and reported as `SkippedModified`, and `Current` is
/// reported as `Unchanged` without touching anything. `Missing` is rewritten
/// only under `MissingPolicy::Restore` — `Preserve` (what the pre-command
/// auto-refresh passes) leaves a deliberately deleted file deleted, reporting
/// nothing and leaving the entry's version untouched so it is not mistaken
/// for current.
///
/// A single entry's write failing (read-only filesystem, permission denied,
/// ...) is reported as `Action::Failed` rather than aborting the loop with
/// `?` — every other entry's refresh still lands, and `save_state` still
/// persists them, so one unwritable path cannot swallow the whole batch nor
/// spam the same warning on every future invocation forever. The failed
/// entry keeps its old version and hash, so it is retried next time rather
/// than being mistaken for current. `refresh_tracked` itself still returns
/// `Err` when `save_state` fails, since at that point the whole operation's
/// work would otherwise be silently lost.
pub fn refresh_tracked(missing: MissingPolicy) -> Result<Vec<Outcome>> {
    let (state, warning) = load_state();
    if let Some(warning) = warning {
        crate::output::warn(&warning);
    }
    let mut outcomes = Vec::new();
    let mut kept: Vec<Entry> = Vec::new();

    for mut entry in state {
        if !is_shaped_like_a_skill_path(&entry.path) {
            crate::output::warn(&format!(
                "refusing to touch {} — does not look like a skill path bb would have written",
                entry.path.display()
            ));
            // Bookkeeping only: this entry is never rewritten, but stamping the
            // running version here still keeps `tracked_version_differs` cheap —
            // without it, one unshaped entry would make every future invocation
            // believe a refresh is due, forever.
            entry.version = env!("CARGO_PKG_VERSION").to_string();
            kept.push(entry);
            continue;
        }
        // A hand-edited or badly-merged state file could name a skill this
        // binary doesn't know. Leave it untouched here; `status` explicitly
        // reports an unknown skill name as `Modified`, since this binary has
        // no wanted content to compare it against or refresh it with.
        let Some(skill) = skill_by_name(&entry.skill) else {
            // Same bookkeeping-only stamp as above, for the same reason.
            entry.version = env!("CARGO_PKG_VERSION").to_string();
            kept.push(entry);
            continue;
        };

        // An entry whose whole directory tree is gone is not a skill waiting to
        // be restored — it is residue from a temp directory or a deleted
        // checkout. Recreating it would materialise a file inside a path nobody
        // asked for, so drop the entry instead. A missing file whose directory
        // still exists is the opposite case and is restored below. The check
        // looks two levels up (past the skill-name folder itself, which
        // `write_file`/`restore_claude_link` happily recreate) so deleting just
        // the one skill's own folder still restores it — only a vanished parent
        // tree above that (the agent's whole `skills/` directory, or higher)
        // counts as residue.
        if entry
            .path
            .parent()
            .and_then(Path::parent)
            .is_some_and(ancestor_is_definitely_gone)
        {
            outcomes.push(Outcome {
                path: entry.path.clone(),
                agent: entry.agent.clone(),
                skill: entry.skill.clone(),
                action: Action::Pruned,
            });
            continue;
        }

        let wanted = content_hash(skill.content.as_bytes());
        let disk_state = state_of(&entry, &wanted);

        // A deliberately deleted file must not come back from an auto-refresh
        // nobody asked for. Skip it entirely: no write, no outcome, no version
        // stamp — stamping would make it read as current when it is not.
        if disk_state == State::Missing && missing == MissingPolicy::Preserve {
            kept.push(entry);
            continue;
        }

        let action = match disk_state {
            // The link is intact — writing to `entry.path` follows it straight
            // into the `.agents` file it points at, refreshing the shared
            // content without disturbing the link itself.
            State::Stale => match write_file(&entry.path, skill.content) {
                Ok(()) => {
                    entry.sha256 = wanted.clone();
                    Action::Refreshed
                }
                Err(_) => Action::Failed,
            },
            // The link (or file) itself is gone. A plain `write_file` here
            // would create a *real* file where a symlink used to be, leaving
            // state still claiming `"symlink"` while disk disagrees. Restore
            // the same kind of thing that used to be there instead.
            State::Missing => {
                let restored: Result<String> = if entry.kind == "symlink" {
                    restore_claude_link(&entry.path, skill)
                } else {
                    write_file(&entry.path, skill.content).map(|()| "file".to_string())
                };
                match restored {
                    Ok(kind) => {
                        entry.kind = kind;
                        entry.sha256 = wanted.clone();
                        Action::Refreshed
                    }
                    Err(_) => Action::Failed,
                }
            }
            State::Modified => Action::SkippedModified,
            State::Current => Action::Unchanged,
        };

        // Every entry we looked at records this build, including one we
        // skipped — except one whose write just failed: its content is
        // genuinely not current, and stamping the version would hide that
        // from the next invocation's check.
        if action != Action::Failed {
            entry.version = env!("CARGO_PKG_VERSION").to_string();
        }

        outcomes.push(Outcome {
            path: entry.path.clone(),
            agent: entry.agent.clone(),
            skill: entry.skill.clone(),
            action,
        });
        kept.push(entry);
    }

    save_state(&kept)?;
    Ok(outcomes)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn bb_skill() -> &'static Skill {
        skill_by_name("bitbucket-cloud").unwrap()
    }

    /// A packaging regression — an added `exclude` entry in Cargo.toml, or a moved
    /// file — must fail the build rather than ship an empty skill.
    #[test]
    fn embedded_skill_is_present_and_has_frontmatter() {
        assert!(!bb_skill().content.trim().is_empty());
        assert!(
            bb_skill().content.starts_with("---"),
            "skill must open with yaml frontmatter"
        );
        assert!(
            bb_skill().content.contains("name: bitbucket-cloud"),
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
                    sha256: content_hash(bb_skill().content.as_bytes()),
                    version: env!("CARGO_PKG_VERSION").into(),
                    skill: "bitbucket-cloud".into(),
                    created: true,
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
                let path = skill_file(root.path(), Agent::Agents, bb_skill());
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                let old = "---\nname: bitbucket-cloud\n---\nold text\n";
                std::fs::write(&path, old).unwrap();
                save_state(&[Entry {
                    path: path.clone(),
                    agent: "agents".into(),
                    kind: "file".into(),
                    sha256: content_hash(old.as_bytes()),
                    version: "0.0.1".into(),
                    skill: "bitbucket-cloud".into(),
                    created: true,
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
                std::fs::write(&path, bb_skill().content).unwrap();
                save_state(&[Entry {
                    path: path.clone(),
                    agent: "agents".into(),
                    kind: "file".into(),
                    sha256: content_hash(bb_skill().content.as_bytes()),
                    version: env!("CARGO_PKG_VERSION").into(),
                    skill: "bitbucket-cloud".into(),
                    created: true,
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
                install(
                    dir.path(),
                    &[Agent::Agents, Agent::Claude],
                    &[bb_skill()],
                    false,
                )
                .unwrap();
                let agents_path = skill_file(dir.path(), Agent::Agents, bb_skill());
                let claude_root = dir.path().join(".claude");
                assert!(agents_path.is_file(), "sanity: agents copy installed");

                let results = uninstall(Some(&claude_root), &[bb_skill()], false).unwrap();
                assert_eq!(results.len(), 1, "only the claude entry was in scope");
                assert_eq!(
                    results[0].2,
                    RemovalOutcome::Removed,
                    "the claude entry should report removed"
                );

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
                let outcomes = install(dir.path(), &[Agent::Agents], &[bb_skill()], false).unwrap();
                assert_eq!(outcomes.len(), 1);
                assert!(matches!(outcomes[0].action, Action::Installed));

                let written =
                    std::fs::read_to_string(skill_file(dir.path(), Agent::Agents, bb_skill()))
                        .unwrap();
                assert_eq!(
                    written,
                    bb_skill().content,
                    "installed content must equal the embedded skill"
                );

                let (state, _) = load_state();
                assert_eq!(state.len(), 1);
                assert_eq!(state[0].sha256, content_hash(bb_skill().content.as_bytes()));
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
                install(dir.path(), &[Agent::Agents], &[bb_skill()], false).unwrap();
                let outcomes = install(dir.path(), &[Agent::Agents], &[bb_skill()], false).unwrap();
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
                install(dir.path(), &[Agent::Agents], &[bb_skill()], false).unwrap();
                let path = skill_file(dir.path(), Agent::Agents, bb_skill());
                std::fs::write(&path, "# my own notes\n").unwrap();

                let outcomes = install(dir.path(), &[Agent::Agents], &[bb_skill()], false).unwrap();
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
                install(dir.path(), &[Agent::Agents], &[bb_skill()], false).unwrap();
                let path = skill_file(dir.path(), Agent::Agents, bb_skill());
                std::fs::write(&path, "# my own notes\n").unwrap();

                let outcomes = install(dir.path(), &[Agent::Agents], &[bb_skill()], true).unwrap();
                assert!(matches!(
                    outcomes[0].action,
                    Action::Refreshed | Action::Installed
                ));
                assert_eq!(std::fs::read_to_string(&path).unwrap(), bb_skill().content);
                let (state, _) = load_state();
                assert_eq!(state[0].sha256, content_hash(bb_skill().content.as_bytes()));
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
                let path = skill_file(dir.path(), Agent::Agents, bb_skill());
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                let old = "---\nname: bitbucket-cloud\n---\nold text\n";
                std::fs::write(&path, old).unwrap();
                save_state(&[Entry {
                    path: path.clone(),
                    agent: "agents".into(),
                    kind: "file".into(),
                    sha256: content_hash(old.as_bytes()),
                    version: "0.0.1".into(),
                    skill: "bitbucket-cloud".into(),
                    created: true,
                }])
                .unwrap();

                let outcomes = install(dir.path(), &[Agent::Agents], &[bb_skill()], false).unwrap();
                assert!(
                    matches!(outcomes[0].action, Action::Refreshed),
                    "{:?}",
                    outcomes[0].action
                );
                assert_eq!(std::fs::read_to_string(&path).unwrap(), bb_skill().content);
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
                install(
                    dir.path(),
                    &[Agent::Agents, Agent::Claude],
                    &[bb_skill()],
                    false,
                )
                .unwrap();
                let claude = dir.path().join(".claude/skills").join(bb_skill().name);
                // Either a symlink resolving to the .agents copy, or a real file with
                // the same content when the platform refused a symlink.
                let content = std::fs::read_to_string(claude.join("SKILL.md"))
                    .or_else(|_| std::fs::read_to_string(&claude))
                    .unwrap();
                assert_eq!(content, bb_skill().content);
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

    /// A deleted Claude symlink with no `.agents` copy to point at falls back
    /// to a real file — same as `install_claude_dir` would on a fresh
    /// install — and the recorded `kind` must follow disk down to `"file"`,
    /// not keep claiming `"symlink"`.
    #[test]
    #[serial_test::serial]
    fn refresh_recreates_a_deleted_symlink_as_a_file_when_no_agents_copy_exists() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = tempfile::tempdir().unwrap();
        temp_env(
            &[
                ("XDG_CONFIG_HOME", Some(cfg.path().to_str().unwrap())),
                ("HOME", None),
            ],
            || {
                install(
                    dir.path(),
                    &[Agent::Agents, Agent::Claude],
                    &[bb_skill()],
                    false,
                )
                .unwrap();
                let claude_path = skill_file(dir.path(), Agent::Claude, bb_skill());
                let claude_dir = claude_path.parent().unwrap();

                // Only the claude entry is tracked, and its target is gone —
                // the state this test wants to force is "link recorded, but
                // nothing left to link to".
                let (state, _) = load_state();
                let claude_entry = state.iter().find(|e| e.agent == "claude").cloned().unwrap();
                assert_eq!(claude_entry.kind, "symlink", "sanity: install made a link");
                save_state(&[claude_entry]).unwrap();

                remove_existing(claude_dir).unwrap();
                std::fs::remove_dir_all(dir.path().join(".agents")).unwrap();

                let outcomes = refresh_tracked(MissingPolicy::Restore).unwrap();
                assert_eq!(outcomes.len(), 1);
                assert!(matches!(outcomes[0].action, Action::Refreshed));

                assert_eq!(
                    std::fs::read_to_string(&claude_path).unwrap(),
                    bb_skill().content
                );
                let (state, _) = load_state();
                assert_eq!(
                    state[0].kind, "file",
                    "disk fell back to a real file, so state must say so too"
                );
            },
        );
    }

    /// A deleted Claude symlink is recreated as a symlink, not a file, when
    /// the `.agents` copy it used to point at is still there.
    #[test]
    #[serial_test::serial]
    fn refresh_recreates_a_deleted_symlink_as_a_symlink_when_the_agents_copy_survives() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = tempfile::tempdir().unwrap();
        temp_env(
            &[
                ("XDG_CONFIG_HOME", Some(cfg.path().to_str().unwrap())),
                ("HOME", None),
            ],
            || {
                install(
                    dir.path(),
                    &[Agent::Agents, Agent::Claude],
                    &[bb_skill()],
                    false,
                )
                .unwrap();
                let claude_dir = skill_file(dir.path(), Agent::Claude, bb_skill())
                    .parent()
                    .unwrap()
                    .to_path_buf();

                remove_existing(&claude_dir).unwrap();
                assert!(!claude_dir.exists(), "sanity: the link is gone");

                let outcomes = refresh_tracked(MissingPolicy::Restore).unwrap();
                let claude_outcome = outcomes.iter().find(|o| o.agent == "claude").unwrap();
                assert!(matches!(claude_outcome.action, Action::Refreshed));

                assert!(
                    std::fs::symlink_metadata(&claude_dir)
                        .unwrap()
                        .file_type()
                        .is_symlink(),
                    "the agents copy was still there, so a link should come back, not a file"
                );
                assert_eq!(
                    std::fs::read_to_string(claude_dir.join("SKILL.md")).unwrap(),
                    bb_skill().content
                );
                let (state, _) = load_state();
                let claude_entry = state.iter().find(|e| e.agent == "claude").unwrap();
                assert_eq!(claude_entry.kind, "symlink");
            },
        );
    }

    /// A stale Claude symlink whose link is still intact refreshes the
    /// shared `.agents` content in place — `write_file` follows the link
    /// rather than replacing it — so the link itself must survive untouched.
    #[test]
    #[serial_test::serial]
    fn refresh_updates_content_through_an_intact_symlink_without_replacing_it() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = tempfile::tempdir().unwrap();
        temp_env(
            &[
                ("XDG_CONFIG_HOME", Some(cfg.path().to_str().unwrap())),
                ("HOME", None),
            ],
            || {
                install(
                    dir.path(),
                    &[Agent::Agents, Agent::Claude],
                    &[bb_skill()],
                    false,
                )
                .unwrap();
                let claude_path = skill_file(dir.path(), Agent::Claude, bb_skill());
                let claude_dir = claude_path.parent().unwrap().to_path_buf();

                let old = "---\nname: bitbucket-cloud\n---\nold text\n";
                std::fs::write(&claude_path, old).unwrap();

                // Only the claude entry is tracked, so the refresh's only
                // write comes from the `Stale` branch on this entry, not from
                // an `.agents` entry rewriting the same underlying file.
                let (state, _) = load_state();
                let mut claude_entry = state.iter().find(|e| e.agent == "claude").cloned().unwrap();
                claude_entry.sha256 = content_hash(old.as_bytes());
                save_state(&[claude_entry]).unwrap();

                let outcomes = refresh_tracked(MissingPolicy::Restore).unwrap();
                assert_eq!(outcomes.len(), 1);
                assert!(matches!(outcomes[0].action, Action::Refreshed));

                assert!(
                    std::fs::symlink_metadata(&claude_dir)
                        .unwrap()
                        .file_type()
                        .is_symlink(),
                    "an intact link must not be replaced by a file just to refresh content"
                );
                assert_eq!(
                    std::fs::read_to_string(&claude_path).unwrap(),
                    bb_skill().content
                );
                let (state, _) = load_state();
                assert_eq!(state[0].kind, "symlink");
            },
        );
    }

    #[test]
    fn shape_guard_accepts_only_the_two_real_skill_locations() {
        assert!(is_shaped_like_a_skill_path(Path::new(
            "/proj/.agents/skills/bitbucket-cloud/SKILL.md"
        )));
        assert!(is_shaped_like_a_skill_path(Path::new(
            "/proj/.claude/skills/bitbucket-cloud/SKILL.md"
        )));
        for bad in [
            "/proj/src",
            "/proj/src/main.rs",
            "/proj/.agents/skills/bitbucket-cloud",
            "/proj/.agents/skills/some-other-skill/SKILL.md",
            "/proj/.opencode/skills/bitbucket-cloud/SKILL.md",
            "/etc/passwd",
        ] {
            assert!(
                !is_shaped_like_a_skill_path(Path::new(bad)),
                "{bad} should have been refused"
            );
        }
    }

    /// Critical 2, reproduced and fixed: a hand-made Claude symlink — exactly
    /// what the old README's `ln -s` instructions told users to create —
    /// must not be deleted-through by `uninstall` just because no prior state
    /// entry existed to tell `install` its `kind` was `"symlink"`.
    #[test]
    #[serial_test::serial]
    fn uninstall_does_not_follow_a_hand_made_symlink_into_its_target() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = tempfile::tempdir().unwrap();
        temp_env(
            &[
                ("XDG_CONFIG_HOME", Some(cfg.path().to_str().unwrap())),
                ("HOME", None),
            ],
            || {
                // Set up the .agents copy the way a real project would have
                // it, then hand-make the Claude symlink exactly as the old
                // README instructed, *before* bb has ever recorded anything.
                let agents_path = skill_file(dir.path(), Agent::Agents, bb_skill());
                std::fs::create_dir_all(agents_path.parent().unwrap()).unwrap();
                std::fs::write(&agents_path, bb_skill().content).unwrap();

                let claude_dir = dir
                    .path()
                    .join(".claude")
                    .join("skills")
                    .join(bb_skill().name);
                std::fs::create_dir_all(claude_dir.parent().unwrap()).unwrap();
                #[cfg(unix)]
                std::os::unix::fs::symlink(
                    Path::new("..")
                        .join("..")
                        .join(".agents")
                        .join("skills")
                        .join(bb_skill().name),
                    &claude_dir,
                )
                .unwrap();

                let outcomes = install(dir.path(), &[Agent::Claude], &[bb_skill()], false).unwrap();
                assert!(
                    matches!(outcomes[0].action, Action::Unchanged),
                    "sanity: content already matches, so install should not rewrite it"
                );

                let results = uninstall(None, &[bb_skill()], false).unwrap();
                assert_eq!(results.len(), 1);
                assert_eq!(results[0].2, RemovalOutcome::Removed);

                assert!(
                    std::fs::read_to_string(&agents_path).unwrap() == bb_skill().content,
                    "the .agents copy must survive uninstall of the claude link"
                );
                assert!(
                    std::fs::symlink_metadata(&claude_dir).is_err(),
                    "no dangling claude symlink should remain (Path::exists() would \
                     wrongly report false for a dangling link, so this checks \
                     symlink_metadata instead)"
                );
            },
        );
    }

    /// Important 3, reproduced and fixed: a state entry pointing at an
    /// unrelated directory must be refused, and that directory must survive.
    #[test]
    #[serial_test::serial]
    fn uninstall_refuses_a_state_entry_pointing_outside_the_skill_shape() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = tempfile::tempdir().unwrap();
        temp_env(
            &[
                ("XDG_CONFIG_HOME", Some(cfg.path().to_str().unwrap())),
                ("HOME", None),
            ],
            || {
                let victim_dir = dir.path().join("src");
                std::fs::create_dir_all(&victim_dir).unwrap();
                std::fs::write(victim_dir.join("main.rs"), "fn main() {}").unwrap();

                let victim_file = dir.path().join("Cargo.toml");
                std::fs::write(&victim_file, "[package]").unwrap();

                save_state(&[
                    Entry {
                        path: victim_dir.clone(),
                        agent: "agents".into(),
                        kind: "file".into(),
                        sha256: "deadbeef".into(),
                        version: "0.0.1".into(),
                        skill: "bitbucket-cloud".into(),
                        created: true,
                    },
                    Entry {
                        path: victim_file.clone(),
                        agent: "agents".into(),
                        kind: "file".into(),
                        sha256: "deadbeef".into(),
                        version: "0.0.1".into(),
                        skill: "bitbucket-cloud".into(),
                        created: true,
                    },
                ])
                .unwrap();

                let results = uninstall(None, &[bb_skill()], true).unwrap();
                assert_eq!(results.len(), 2);
                assert!(results
                    .iter()
                    .all(|(_, _, o)| *o == RemovalOutcome::RefusedUnsafePath));

                assert!(victim_dir.is_dir(), "unrelated directory must survive");
                assert!(
                    victim_dir.join("main.rs").exists(),
                    "unrelated directory's contents must survive"
                );
                assert!(victim_file.is_file(), "unrelated file must survive");

                // Refused entries stay tracked rather than being dropped.
                let (remaining, _) = load_state();
                assert_eq!(remaining.len(), 2);
            },
        );
    }

    /// A legitimate entry still works after the shape guard was added.
    #[test]
    #[serial_test::serial]
    fn uninstall_still_removes_a_legitimate_entry() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = tempfile::tempdir().unwrap();
        temp_env(
            &[
                ("XDG_CONFIG_HOME", Some(cfg.path().to_str().unwrap())),
                ("HOME", None),
            ],
            || {
                install(dir.path(), &[Agent::Agents], &[bb_skill()], false).unwrap();
                let results = uninstall(None, &[bb_skill()], false).unwrap();
                assert_eq!(
                    results,
                    vec![(
                        skill_file(dir.path(), Agent::Agents, bb_skill()),
                        bb_skill().name.to_string(),
                        RemovalOutcome::Removed
                    )]
                );
            },
        );
    }

    /// The bug from #50, and the invariant `uninstall`'s own doc comment already
    /// claims: "an untracked file is never touched at all".
    ///
    /// `install` decided what to record from a content hash alone. A hash match
    /// proves the bytes are identical, never that `bb` is what put them there,
    /// so a `SKILL.md` that already existed and happened to match was reported
    /// `Unchanged` — accurate, `bb` wrote nothing — and then recorded as a file
    /// `bb` owns. The next `uninstall` deleted it.
    ///
    /// The sharp case is this crate's own checkout, where `.agents/skills/*/SKILL.md`
    /// are the tracked sources `include_str!` compiles in. They match the embedded
    /// copies by construction, so `install` claimed all of them and `uninstall`
    /// deleted the crate's own sources, breaking the build.
    #[test]
    #[serial_test::serial]
    fn uninstall_leaves_a_file_bb_never_wrote() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = tempfile::tempdir().unwrap();
        temp_env(
            &[
                ("XDG_CONFIG_HOME", Some(cfg.path().to_str().unwrap())),
                ("HOME", None),
            ],
            || {
                // Someone else's file, byte-identical to what bb ships — a
                // vendored copy, or this crate's own source tree.
                let path = skill_file(dir.path(), Agent::Agents, bb_skill());
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(&path, bb_skill().content).unwrap();

                let installed =
                    install(dir.path(), &[Agent::Agents], &[bb_skill()], false).unwrap();
                assert_eq!(
                    installed[0].action,
                    Action::Unchanged,
                    "bb wrote nothing, so the action is still Unchanged"
                );

                let results = uninstall(None, &[bb_skill()], false).unwrap();
                assert_eq!(
                    results,
                    vec![(
                        path.clone(),
                        bb_skill().name.to_string(),
                        RemovalOutcome::RefusedNotWritten
                    )]
                );
                assert!(
                    path.exists(),
                    "uninstall deleted a file bb never wrote: {}",
                    path.display()
                );

                // "Uninstall" still means stop managing it, so the entry is
                // dropped — otherwise every later run re-reports the refusal.
                let (entries, _) = load_state();
                assert!(
                    !entries.iter().any(|e| e.path == path),
                    "the adopted entry should be untracked after uninstall"
                );
            },
        );
    }

    /// The escape hatch. `--force` already means "remove it even though I would
    /// normally refuse", and someone who vendored a copy and then asked for it
    /// to go, explicitly, gets what they asked for.
    #[test]
    #[serial_test::serial]
    fn force_removes_a_file_bb_never_wrote() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = tempfile::tempdir().unwrap();
        temp_env(
            &[
                ("XDG_CONFIG_HOME", Some(cfg.path().to_str().unwrap())),
                ("HOME", None),
            ],
            || {
                let path = skill_file(dir.path(), Agent::Agents, bb_skill());
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(&path, bb_skill().content).unwrap();
                install(dir.path(), &[Agent::Agents], &[bb_skill()], false).unwrap();

                let results = uninstall(None, &[bb_skill()], true).unwrap();
                assert_eq!(results[0].2, RemovalOutcome::Removed);
                assert!(!path.exists());
            },
        );
    }

    /// A file bb wrote itself, then found unchanged on a second install, stays
    /// removable. Provenance has to survive the `Unchanged` path, or the fix
    /// above would quietly strand every skill after its second install.
    #[test]
    #[serial_test::serial]
    fn a_reinstalled_file_is_still_removable() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = tempfile::tempdir().unwrap();
        temp_env(
            &[
                ("XDG_CONFIG_HOME", Some(cfg.path().to_str().unwrap())),
                ("HOME", None),
            ],
            || {
                install(dir.path(), &[Agent::Agents], &[bb_skill()], false).unwrap();
                let again = install(dir.path(), &[Agent::Agents], &[bb_skill()], false).unwrap();
                assert_eq!(again[0].action, Action::Unchanged);

                let results = uninstall(None, &[bb_skill()], false).unwrap();
                assert_eq!(results[0].2, RemovalOutcome::Removed);
            },
        );
    }

    /// State files written before this field existed carry no `created`. They
    /// were, overwhelmingly, written by `bb` — so they default to true and stay
    /// removable, rather than stranding every skill installed before the upgrade.
    #[test]
    #[serial_test::serial]
    fn a_legacy_state_entry_without_the_field_is_still_removable() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = tempfile::tempdir().unwrap();
        temp_env(
            &[
                ("XDG_CONFIG_HOME", Some(cfg.path().to_str().unwrap())),
                ("HOME", None),
            ],
            || {
                let path = skill_file(dir.path(), Agent::Agents, bb_skill());
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(&path, bb_skill().content).unwrap();

                let legacy = serde_json::json!([{
                    "path": path,
                    "agent": "agents",
                    "kind": "file",
                    "sha256": content_hash(bb_skill().content.as_bytes()),
                    "version": "0.18.0",
                    "skill": bb_skill().name,
                }]);
                std::fs::create_dir_all(cfg.path().join("bb")).unwrap();
                std::fs::write(
                    cfg.path().join("bb").join("skills.json"),
                    serde_json::to_string(&legacy).unwrap(),
                )
                .unwrap();

                let results = uninstall(None, &[bb_skill()], false).unwrap();
                assert_eq!(results[0].2, RemovalOutcome::Removed);
            },
        );
    }

    /// The design's core claim: one customized skill must not block another
    /// tracked skill's refresh, and the skipped one must be named in the
    /// output rather than silently dropped.
    #[test]
    #[serial_test::serial]
    fn refresh_rewrites_a_stale_entry_while_leaving_a_modified_one_alone() {
        let stale_root = tempfile::tempdir().unwrap();
        let modified_root = tempfile::tempdir().unwrap();
        let cfg = tempfile::tempdir().unwrap();
        temp_env(
            &[
                ("XDG_CONFIG_HOME", Some(cfg.path().to_str().unwrap())),
                ("HOME", None),
            ],
            || {
                let old = "---\nname: bitbucket-cloud\n---\nold text\n";
                let stale_path = skill_file(stale_root.path(), Agent::Agents, bb_skill());
                std::fs::create_dir_all(stale_path.parent().unwrap()).unwrap();
                std::fs::write(&stale_path, old).unwrap();

                let modified_path = skill_file(modified_root.path(), Agent::Agents, bb_skill());
                std::fs::create_dir_all(modified_path.parent().unwrap()).unwrap();
                let ours = "# our own version\n";
                std::fs::write(&modified_path, ours).unwrap();

                save_state(&[
                    Entry {
                        path: stale_path.clone(),
                        agent: "agents".into(),
                        kind: "file".into(),
                        sha256: content_hash(old.as_bytes()),
                        version: "0.0.1".into(),
                        skill: "bitbucket-cloud".into(),
                        created: true,
                    },
                    Entry {
                        path: modified_path.clone(),
                        agent: "agents".into(),
                        kind: "file".into(),
                        // Recorded hash disagrees with what's on disk now —
                        // someone edited it after bb wrote it.
                        sha256: content_hash(old.as_bytes()),
                        version: "0.0.1".into(),
                        skill: "bitbucket-cloud".into(),
                        created: true,
                    },
                ])
                .unwrap();

                let outcomes = refresh_tracked(MissingPolicy::Restore).unwrap();
                assert_eq!(outcomes.len(), 2);

                let stale_outcome = outcomes.iter().find(|o| o.path == stale_path).unwrap();
                assert!(matches!(stale_outcome.action, Action::Refreshed));
                assert_eq!(
                    std::fs::read_to_string(&stale_path).unwrap(),
                    bb_skill().content
                );

                let modified_outcome = outcomes.iter().find(|o| o.path == modified_path).unwrap();
                assert!(matches!(modified_outcome.action, Action::SkippedModified));
                assert_eq!(std::fs::read_to_string(&modified_path).unwrap(), ours);
            },
        );
    }

    #[test]
    fn both_skills_are_registered_and_well_formed() {
        let names: Vec<&str> = SKILLS.iter().map(|s| s.name).collect();
        assert!(names.contains(&"bitbucket-cloud"));
        assert!(names.contains(&"bbc-daily-brief"));
        assert!(names.contains(&"bbc-open-pr"));
        assert!(names.contains(&"bbc-report-bug"));
        for skill in SKILLS.iter() {
            assert!(
                skill.content.starts_with("---"),
                "{} lacks frontmatter",
                skill.name
            );
            assert!(
                skill.content.contains(&format!("name: {}", skill.name)),
                "{} frontmatter name does not match",
                skill.name
            );
        }
    }

    /// The bug-reporting skill drives `gh issue create`, which publishes to a
    /// public repository and cannot be undone. Two clauses are what stop it
    /// leaking the user's private Bitbucket data or filing uninvited, and both
    /// have been deleted by well-meaning edits in similar skills before: the
    /// approval gate before the issue is created, and the redaction of
    /// workspace and repository names. A skill file is prose, so a test can
    /// only hold the load-bearing phrases in place — but that is exactly the
    /// part a summarising edit drops first.
    #[test]
    fn the_bug_report_skill_keeps_its_approval_gate_and_redaction_rules() {
        let skill = skill_by_name("bbc-report-bug").expect("bbc-report-bug is registered");

        for required in [
            // Never files uninvited.
            "Explicit invocation only",
            // Shows the draft and waits, before anything is created.
            "Never create the issue without showing it first",
            // Private names never reach a public issue.
            "Redact before you draft",
            // The one value that must never appear at all.
            "never include, redacted or not",
            // A single hardcoded target, so it cannot be pointed elsewhere.
            "biokraft/bbcloud",
        ] {
            assert!(
                skill.content.contains(required),
                "bbc-report-bug lost the {required:?} rule"
            );
        }
    }

    #[test]
    fn skill_by_name_resolves_known_and_rejects_unknown() {
        assert_eq!(
            skill_by_name("bbc-daily-brief").map(|s| s.name),
            Some("bbc-daily-brief")
        );
        assert!(skill_by_name("nope").is_none());
    }

    #[test]
    fn every_skill_path_shape_is_accepted_under_both_layouts() {
        for skill in SKILLS.iter() {
            for dir in [".agents", ".claude"] {
                let path = PathBuf::from(format!("/p/{dir}/skills/{}/SKILL.md", skill.name));
                assert!(is_shaped_like_a_skill_path(&path), "rejected {path:?}");
            }
        }
    }

    #[test]
    fn an_unknown_skill_name_is_still_refused() {
        assert!(!is_shaped_like_a_skill_path(&PathBuf::from(
            "/p/.agents/skills/other-skill/SKILL.md"
        )));
        assert!(!is_shaped_like_a_skill_path(&PathBuf::from(
            "/p/.agents/bbc-daily-brief/SKILL.md"
        )));
        assert!(!is_shaped_like_a_skill_path(&PathBuf::from(
            "/p/.vscode/skills/bbc-daily-brief/SKILL.md"
        )));
    }

    #[test]
    fn an_entry_without_a_skill_field_defaults_to_the_first_skill() {
        let entry: Entry = serde_json::from_str(
            r#"{"path":"/p/.agents/skills/bitbucket-cloud/SKILL.md","agent":"agents",
                "kind":"file","sha256":"abc","version":"0.1.0"}"#,
        )
        .unwrap();
        assert_eq!(entry.skill, "bitbucket-cloud");
    }

    #[test]
    fn state_of_compares_against_each_entrys_own_skill() {
        // A daily-brief file holding daily-brief content is Current, even though
        // it does not match the bitbucket-cloud hash.
        let brief = skill_by_name("bbc-daily-brief").unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("SKILL.md");
        std::fs::write(&path, brief.content).unwrap();
        let entry = Entry {
            path: path.clone(),
            agent: "agents".into(),
            kind: "file".into(),
            sha256: content_hash(brief.content.as_bytes()),
            version: "0.1.0".into(),
            skill: "bbc-daily-brief".into(),
            created: true,
        };
        assert_eq!(
            state_of(&entry, &content_hash(brief.content.as_bytes())),
            State::Current
        );
    }

    /// An entry naming a skill this binary does not know — a `skills.json`
    /// written by a newer `bb`, or a hand-edited file — must never read as
    /// `Stale`: that state promises "the binary has newer text, a refresh
    /// will fix it", which is not true when there is no wanted content to
    /// compare against at all. It must report `Modified` so nothing rewrites
    /// or removes it.
    #[test]
    #[serial_test::serial]
    fn status_reports_modified_for_an_unknown_skill_name_even_when_disk_matches_the_recorded_hash()
    {
        let dir = tempfile::tempdir().unwrap();
        let cfg = tempfile::tempdir().unwrap();
        temp_env(
            &[
                ("XDG_CONFIG_HOME", Some(cfg.path().to_str().unwrap())),
                ("HOME", None),
            ],
            || {
                let path = dir.path().join(".agents/skills/some-future-skill/SKILL.md");
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                let content = "---\nname: some-future-skill\n---\nfrom a newer bb\n";
                std::fs::write(&path, content).unwrap();

                save_state(&[Entry {
                    path: path.clone(),
                    agent: "agents".into(),
                    kind: "file".into(),
                    // On-disk content hashes equal to the recorded sha256 —
                    // exactly what would make a *known* skill read as Stale.
                    sha256: content_hash(content.as_bytes()),
                    version: "9.9.9".into(),
                    skill: "some-future-skill".into(),
                    created: true,
                }])
                .unwrap();

                let (rows, warning) = status();
                assert!(warning.is_none());
                assert_eq!(rows.len(), 1);
                assert_eq!(
                    rows[0].state,
                    State::Modified,
                    "an unknown skill name must never be reported as Stale"
                );
            },
        );
    }

    #[test]
    fn tracked_version_differs_only_when_an_entry_is_behind() {
        let current = env!("CARGO_PKG_VERSION").to_string();
        assert!(
            !tracked_version_differs(&[]),
            "nothing tracked means nothing to do"
        );

        let up_to_date = Entry {
            path: PathBuf::from("/p/.agents/skills/bitbucket-cloud/SKILL.md"),
            agent: "agents".into(),
            kind: "file".into(),
            sha256: "abc".into(),
            version: current.clone(),
            skill: "bitbucket-cloud".into(),
            created: true,
        };
        assert!(!tracked_version_differs(std::slice::from_ref(&up_to_date)));

        let mut behind = up_to_date.clone();
        behind.version = "0.0.1".into();
        assert!(tracked_version_differs(&[up_to_date, behind]));
    }

    #[test]
    #[serial_test::serial]
    fn refresh_prunes_an_entry_whose_directory_tree_is_gone() {
        let cfg = tempfile::tempdir().unwrap();
        temp_env(
            &[
                ("XDG_CONFIG_HOME", Some(cfg.path().to_str().unwrap())),
                ("HOME", None),
            ],
            || {
                // A path under a directory that does not exist — the shape a
                // temp-directory install leaves behind once the temp dir is gone.
                let gone =
                    PathBuf::from("/nonexistent-root-xyz/.agents/skills/bitbucket-cloud/SKILL.md");
                save_state(&[Entry {
                    path: gone.clone(),
                    agent: "agents".into(),
                    kind: "file".into(),
                    sha256: "abc".into(),
                    version: "0.0.1".into(),
                    skill: "bitbucket-cloud".into(),
                    created: true,
                }])
                .unwrap();

                let outcomes = refresh_tracked(MissingPolicy::Restore).unwrap();
                assert_eq!(outcomes.len(), 1);
                assert_eq!(outcomes[0].action, Action::Pruned);
                assert!(!gone.exists(), "pruning must not create the file");

                let (state, _) = load_state();
                assert!(
                    state.is_empty(),
                    "the pruned entry must leave the state file"
                );
            },
        );
    }

    #[test]
    #[serial_test::serial]
    fn refresh_still_restores_a_missing_file_whose_directory_exists() {
        let cfg = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        temp_env(
            &[
                ("XDG_CONFIG_HOME", Some(cfg.path().to_str().unwrap())),
                ("HOME", None),
            ],
            || {
                let path = skill_file(root.path(), Agent::Agents, bb_skill());
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                // Directory exists, file does not: someone deleted a skill and
                // wants it back. This must not be confused with a pruned tree.
                save_state(&[Entry {
                    path: path.clone(),
                    agent: "agents".into(),
                    kind: "file".into(),
                    sha256: content_hash(bb_skill().content.as_bytes()),
                    version: "0.0.1".into(),
                    skill: "bitbucket-cloud".into(),
                    created: true,
                }])
                .unwrap();

                let outcomes = refresh_tracked(MissingPolicy::Restore).unwrap();
                assert_eq!(outcomes[0].action, Action::Refreshed);
                assert_eq!(std::fs::read_to_string(&path).unwrap(), bb_skill().content);
            },
        );
    }

    #[test]
    #[serial_test::serial]
    fn refresh_stamps_the_version_onto_a_skipped_entry_without_touching_the_file() {
        let cfg = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        temp_env(
            &[
                ("XDG_CONFIG_HOME", Some(cfg.path().to_str().unwrap())),
                ("HOME", None),
            ],
            || {
                let path = skill_file(root.path(), Agent::Agents, bb_skill());
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(&path, "locally edited").unwrap();
                save_state(&[Entry {
                    path: path.clone(),
                    agent: "agents".into(),
                    kind: "file".into(),
                    sha256: content_hash(b"something else"),
                    version: "0.0.1".into(),
                    skill: "bitbucket-cloud".into(),
                    created: true,
                }])
                .unwrap();

                let outcomes = refresh_tracked(MissingPolicy::Restore).unwrap();
                assert_eq!(outcomes[0].action, Action::SkippedModified);
                assert_eq!(
                    std::fs::read_to_string(&path).unwrap(),
                    "locally edited",
                    "a local edit must survive"
                );

                // The version moves forward even though the file was left alone,
                // so the auto-refresh check does not re-fire on every command.
                let (state, _) = load_state();
                assert_eq!(state[0].version, env!("CARGO_PKG_VERSION"));
                assert!(!tracked_version_differs(&state));
            },
        );
    }

    #[test]
    #[serial_test::serial]
    fn refresh_stamps_the_version_onto_an_unshaped_path_without_creating_it() {
        let cfg = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        temp_env(
            &[
                ("XDG_CONFIG_HOME", Some(cfg.path().to_str().unwrap())),
                ("HOME", None),
            ],
            || {
                // A file named `NOTES.md` instead of `SKILL.md` fails
                // `is_shaped_like_a_skill_path`'s filename check, so this entry
                // hits the shape-guard skip branch.
                let path = root
                    .path()
                    .join(".agents")
                    .join("skills")
                    .join("bitbucket-cloud")
                    .join("NOTES.md");
                save_state(&[Entry {
                    path: path.clone(),
                    agent: "agents".into(),
                    kind: "file".into(),
                    sha256: "abc".into(),
                    version: "0.0.1".into(),
                    skill: "bitbucket-cloud".into(),
                    created: true,
                }])
                .unwrap();

                let outcomes = refresh_tracked(MissingPolicy::Restore).unwrap();
                assert_eq!(
                    outcomes.len(),
                    0,
                    "the shape guard never produces an outcome"
                );
                assert!(!path.exists(), "pruning/stamping must not create the file");

                let (state, _) = load_state();
                assert_eq!(state.len(), 1, "the unshaped entry stays tracked");
                assert_eq!(state[0].version, env!("CARGO_PKG_VERSION"));
                assert!(!tracked_version_differs(&state));
            },
        );
    }

    #[test]
    #[serial_test::serial]
    fn refresh_stamps_the_version_onto_an_unknown_skill_name_without_touching_the_file() {
        let cfg = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        temp_env(
            &[
                ("XDG_CONFIG_HOME", Some(cfg.path().to_str().unwrap())),
                ("HOME", None),
            ],
            || {
                // The path's skill-name directory ("bitbucket-cloud") is one
                // `is_shaped_like_a_skill_path` recognises, so the entry clears
                // the shape guard; it is the `skill` field naming a skill this
                // binary has never heard of that routes it into the
                // unknown-skill-name skip branch.
                let path = skill_file(root.path(), Agent::Agents, bb_skill());
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(&path, "whatever was here").unwrap();
                save_state(&[Entry {
                    path: path.clone(),
                    agent: "agents".into(),
                    kind: "file".into(),
                    sha256: "abc".into(),
                    version: "0.0.1".into(),
                    skill: "some-future-skill".into(),
                    created: true,
                }])
                .unwrap();

                let outcomes = refresh_tracked(MissingPolicy::Restore).unwrap();
                assert_eq!(
                    outcomes.len(),
                    0,
                    "the unknown-skill skip never produces an outcome"
                );
                assert_eq!(
                    std::fs::read_to_string(&path).unwrap(),
                    "whatever was here",
                    "the file must be left alone"
                );

                let (state, _) = load_state();
                assert_eq!(state.len(), 1, "the unknown-skill entry stays tracked");
                assert_eq!(state[0].version, env!("CARGO_PKG_VERSION"));
                assert!(!tracked_version_differs(&state));
            },
        );
    }

    /// Finding 1: `Path::exists()` reads false both for "truly gone" and for
    /// "can't tell" (EACCES on a parent component, an unmounted volume). Only
    /// the first must prune. A genuinely absent path is the case the prune
    /// path exists for at all.
    #[test]
    fn ancestor_is_definitely_gone_is_true_only_for_not_found() {
        assert!(
            ancestor_is_definitely_gone(Path::new("/definitely/does/not/exist/anywhere-xyz")),
            "a path with no such component must read as definitely gone"
        );

        let dir = tempfile::tempdir().unwrap();
        assert!(
            !ancestor_is_definitely_gone(dir.path()),
            "an existing directory must not read as gone"
        );
    }

    /// The "cannot tell" half of finding 1: a permission error on the ancestor
    /// itself must not be treated as "gone" — pruning here would drop a still
    /// -real entry from the state file while the file stays on disk,
    /// untracked forever. Exercised as a real EACCES rather than mocked,
    /// since the predicate takes a `Path` and the OS is the one thing that can
    /// hand back that exact error kind. Skipped when running as root, since
    /// root ignores directory permission bits and the test would otherwise
    /// silently pass for the wrong reason.
    #[test]
    #[cfg(unix)]
    fn ancestor_is_definitely_gone_is_false_for_a_permission_error() {
        use std::os::unix::fs::PermissionsExt;

        let parent = tempfile::tempdir().unwrap();
        let locked = parent.path().join("locked");
        std::fs::create_dir(&locked).unwrap();
        let target = locked.join("child");
        std::fs::create_dir(&target).unwrap();

        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();

        // If running as root, permission bits are ignored and the read
        // succeeds — in that case there is nothing this test can prove, so
        // skip rather than assert something that isn't actually testing the
        // permission-denied path.
        let stat_result = std::fs::symlink_metadata(&target);
        let restore = || {
            let _ = std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755));
        };
        if stat_result.is_ok() {
            restore();
            return;
        }
        let is_permission_error = matches!(
            &stat_result,
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied
        );
        if !is_permission_error {
            // Some other error (unexpected) — restore perms and bail rather
            // than assert on an error shape this test wasn't written for.
            restore();
            return;
        }

        assert!(
            !ancestor_is_definitely_gone(&target),
            "a permission error must not be treated as definitely gone"
        );
        restore();
    }

    /// Finding 2: one entry's write failing must not abort the batch, must
    /// leave that entry's version and hash untouched (so it is retried), and
    /// must not stop `save_state` from persisting the entries that *did*
    /// refresh in the same call.
    #[test]
    #[serial_test::serial]
    #[cfg(unix)]
    fn refresh_reports_failed_for_an_unwritable_entry_without_blocking_the_rest() {
        use std::os::unix::fs::PermissionsExt;

        let writable_root = tempfile::tempdir().unwrap();
        let locked_root = tempfile::tempdir().unwrap();
        let cfg = tempfile::tempdir().unwrap();
        temp_env(
            &[
                ("XDG_CONFIG_HOME", Some(cfg.path().to_str().unwrap())),
                ("HOME", None),
            ],
            || {
                let old = "---\nname: bitbucket-cloud\n---\nold text\n";

                let writable_path = skill_file(writable_root.path(), Agent::Agents, bb_skill());
                std::fs::create_dir_all(writable_path.parent().unwrap()).unwrap();
                std::fs::write(&writable_path, old).unwrap();

                let locked_path = skill_file(locked_root.path(), Agent::Agents, bb_skill());
                std::fs::create_dir_all(locked_path.parent().unwrap()).unwrap();
                std::fs::write(&locked_path, old).unwrap();
                // Lock the skill's own directory so writing SKILL.md inside it
                // fails with EACCES, without needing root or a real read-only
                // filesystem.
                std::fs::set_permissions(
                    locked_path.parent().unwrap(),
                    std::fs::Permissions::from_mode(0o000),
                )
                .unwrap();

                let old_hash = content_hash(old.as_bytes());
                save_state(&[
                    Entry {
                        path: writable_path.clone(),
                        agent: "agents".into(),
                        kind: "file".into(),
                        sha256: old_hash.clone(),
                        version: "0.0.1".into(),
                        skill: "bitbucket-cloud".into(),
                        created: true,
                    },
                    Entry {
                        path: locked_path.clone(),
                        agent: "agents".into(),
                        kind: "file".into(),
                        sha256: old_hash.clone(),
                        version: "0.0.1".into(),
                        skill: "bitbucket-cloud".into(),
                        created: true,
                    },
                ])
                .unwrap();

                let restore = || {
                    let _ = std::fs::set_permissions(
                        locked_path.parent().unwrap(),
                        std::fs::Permissions::from_mode(0o755),
                    );
                };

                // Running as root ignores the permission bits entirely, so the
                // "locked" write would silently succeed — nothing this test
                // can prove in that environment.
                if std::fs::write(locked_path.parent().unwrap().join("probe"), "x").is_ok() {
                    let _ = std::fs::remove_file(locked_path.parent().unwrap().join("probe"));
                    restore();
                    return;
                }

                let result = refresh_tracked(MissingPolicy::Restore);
                restore();
                let outcomes = result.unwrap();

                assert_eq!(outcomes.len(), 2);
                let writable_outcome = outcomes.iter().find(|o| o.path == writable_path).unwrap();
                assert_eq!(writable_outcome.action, Action::Refreshed);
                assert_eq!(
                    std::fs::read_to_string(&writable_path).unwrap(),
                    bb_skill().content,
                    "the writable entry must still refresh despite the other one failing"
                );

                let locked_outcome = outcomes.iter().find(|o| o.path == locked_path).unwrap();
                assert_eq!(locked_outcome.action, Action::Failed);

                let (state, _) = load_state();
                let writable_entry = state.iter().find(|e| e.path == writable_path).unwrap();
                assert_eq!(
                    writable_entry.version,
                    env!("CARGO_PKG_VERSION"),
                    "save_state must have persisted the entry that did succeed"
                );
                let locked_entry = state.iter().find(|e| e.path == locked_path).unwrap();
                assert_eq!(
                    locked_entry.version, "0.0.1",
                    "a failed write must not stamp the version — it is not current"
                );
                assert_eq!(
                    locked_entry.sha256, old_hash,
                    "a failed write must not update the recorded hash either"
                );
            },
        );
    }

    /// Finding 4, unit-level: `MissingPolicy::Preserve` must not restore a
    /// missing file, must not report an outcome for it, and must not stamp
    /// its version — all three, or the entry would look "handled" when
    /// nothing happened. `Restore` on the same fixture is already proven by
    /// `refresh_still_restores_a_missing_file_whose_directory_exists`.
    #[test]
    #[serial_test::serial]
    fn refresh_with_preserve_leaves_a_missing_file_missing() {
        let cfg = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        temp_env(
            &[
                ("XDG_CONFIG_HOME", Some(cfg.path().to_str().unwrap())),
                ("HOME", None),
            ],
            || {
                let path = skill_file(root.path(), Agent::Agents, bb_skill());
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                save_state(&[Entry {
                    path: path.clone(),
                    agent: "agents".into(),
                    kind: "file".into(),
                    sha256: content_hash(bb_skill().content.as_bytes()),
                    version: "0.0.1".into(),
                    skill: "bitbucket-cloud".into(),
                    created: true,
                }])
                .unwrap();

                let outcomes = refresh_tracked(MissingPolicy::Preserve).unwrap();
                assert!(
                    outcomes.is_empty(),
                    "a preserved missing entry must produce no outcome"
                );
                assert!(!path.exists(), "the file must stay deleted");

                let (state, _) = load_state();
                assert_eq!(state.len(), 1, "the entry stays tracked");
                assert_eq!(
                    state[0].version, "0.0.1",
                    "an untouched entry must not be stamped as current"
                );
            },
        );
    }
}
