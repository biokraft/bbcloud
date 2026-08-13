#![allow(clippy::unwrap_used)] // test code is exempt from the unwrap/expect ban

use assert_cmd::Command;
use predicates::prelude::PredicateBooleanExt;
use predicates::str::contains;

/// Runs `bb` with the project root and both config locations pointed inside
/// tempdirs, so a test can never write the developer's real `~/.config/bb` or
/// reach the real OS keyring.
fn bb(project: &std::path::Path, cfg: &std::path::Path) -> Command {
    let mut cmd = Command::cargo_bin("bb").unwrap();
    cmd.current_dir(project)
        .env("HOME", cfg)
        .env("XDG_CONFIG_HOME", cfg)
        .env("NO_COLOR", "1")
        // These commands never touch the keyring by design; this is belt and braces.
        .env("BB_KEYRING_DISABLE", "1");
    cmd
}

/// Convenience over `bb()` for tests that don't need the project root and the
/// config location to be separate tempdirs.
fn bb_in(dir: &std::path::Path) -> Command {
    bb(dir, dir)
}

#[test]
fn install_creates_the_agents_skill_and_says_so() {
    let project = tempfile::tempdir().unwrap();
    let cfg = tempfile::tempdir().unwrap();

    bb(project.path(), cfg.path())
        .args(["skill", "install"])
        .assert()
        .success()
        .stdout(contains(".agents/skills/bitbucket-cloud/SKILL.md"));

    let installed = project
        .path()
        .join(".agents/skills/bitbucket-cloud/SKILL.md");
    assert!(installed.is_file(), "skill was not written");
    let text = std::fs::read_to_string(installed).unwrap();
    assert!(text.starts_with("---"), "installed file is not the skill");
}

/// The whole point of the group: it must work on a machine that has never run
/// `bb auth login`. Anything routed through the credential loader exits 2.
#[test]
fn install_needs_no_credentials() {
    let project = tempfile::tempdir().unwrap();
    let cfg = tempfile::tempdir().unwrap();

    bb(project.path(), cfg.path())
        .args(["skill", "install"])
        .env_remove("BB_EMAIL")
        .env_remove("BB_TOKEN")
        .assert()
        .success();
}

#[test]
fn install_is_idempotent() {
    let project = tempfile::tempdir().unwrap();
    let cfg = tempfile::tempdir().unwrap();
    bb(project.path(), cfg.path())
        .args(["skill", "install"])
        .assert()
        .success();

    bb(project.path(), cfg.path())
        .args(["skill", "install"])
        .assert()
        .success()
        .stdout(contains("unchanged"));
}

#[test]
fn a_modified_skill_makes_install_exit_one_without_clobbering() {
    let project = tempfile::tempdir().unwrap();
    let cfg = tempfile::tempdir().unwrap();
    bb(project.path(), cfg.path())
        .args(["skill", "install"])
        .assert()
        .success();

    let path = project
        .path()
        .join(".agents/skills/bitbucket-cloud/SKILL.md");
    std::fs::write(&path, "# ours\n").unwrap();

    bb(project.path(), cfg.path())
        .args(["skill", "install"])
        .assert()
        .code(1)
        .stdout(contains("skipped_modified").not())
        .stderr(contains("skipped_modified"))
        .stderr(contains("--force"));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "# ours\n");

    bb(project.path(), cfg.path())
        .args(["skill", "install", "--force"])
        .assert()
        .success();
    assert!(std::fs::read_to_string(&path).unwrap().starts_with("---"));
}

#[test]
fn status_reports_current_then_modified() {
    let project = tempfile::tempdir().unwrap();
    let cfg = tempfile::tempdir().unwrap();
    bb(project.path(), cfg.path())
        .args(["skill", "install"])
        .assert()
        .success();

    bb(project.path(), cfg.path())
        .args(["skill", "status"])
        .assert()
        .success()
        .stdout(contains("current"));

    std::fs::write(
        project
            .path()
            .join(".agents/skills/bitbucket-cloud/SKILL.md"),
        "# ours\n",
    )
    .unwrap();

    bb(project.path(), cfg.path())
        .args(["skill", "status"])
        .assert()
        .success()
        // `contains("modified")` would also match `skipped_modified` — the
        // glyph that matters here is the bare `State::Modified` word, not a
        // substring of some other state's name.
        .stdout(contains("modified").and(contains("skipped_modified").not()));
}

#[test]
fn status_reports_missing_when_the_file_is_deleted() {
    let project = tempfile::tempdir().unwrap();
    let cfg = tempfile::tempdir().unwrap();
    bb(project.path(), cfg.path())
        .args(["skill", "install"])
        .assert()
        .success();
    std::fs::remove_file(
        project
            .path()
            .join(".agents/skills/bitbucket-cloud/SKILL.md"),
    )
    .unwrap();

    bb(project.path(), cfg.path())
        .args(["skill", "status"])
        .assert()
        .success()
        .stdout(contains("missing"));
}

#[test]
fn uninstall_removes_the_file_and_forgets_it() {
    let project = tempfile::tempdir().unwrap();
    let cfg = tempfile::tempdir().unwrap();
    bb(project.path(), cfg.path())
        .args(["skill", "install"])
        .assert()
        .success();

    bb(project.path(), cfg.path())
        .args(["skill", "uninstall"])
        .assert()
        .success();
    assert!(!project
        .path()
        .join(".agents/skills/bitbucket-cloud/SKILL.md")
        .exists());

    // Nothing tracked any more.
    let out = bb(project.path(), cfg.path())
        .args(["skill", "status", "--json"])
        .output()
        .unwrap();
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(value, serde_json::json!([]));
}

#[test]
fn uninstall_leaves_a_modified_file_alone_without_force() {
    let project = tempfile::tempdir().unwrap();
    let cfg = tempfile::tempdir().unwrap();
    bb(project.path(), cfg.path())
        .args(["skill", "install"])
        .assert()
        .success();
    let path = project
        .path()
        .join(".agents/skills/bitbucket-cloud/SKILL.md");
    std::fs::write(&path, "# ours\n").unwrap();

    bb(project.path(), cfg.path())
        .args(["skill", "uninstall"])
        .assert()
        .success();
    assert!(
        path.exists(),
        "a customized skill must not be deleted silently"
    );
}

#[test]
fn json_output_is_pure_on_every_subcommand() {
    let project = tempfile::tempdir().unwrap();
    let cfg = tempfile::tempdir().unwrap();

    for args in [
        vec!["skill", "status", "--json"],
        vec!["skill", "install", "--json"],
        vec!["skill", "status", "--json"],
        vec!["skill", "uninstall", "--json"],
    ] {
        let out = bb(project.path(), cfg.path()).args(&args).output().unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        serde_json::from_str::<serde_json::Value>(stdout.trim())
            .unwrap_or_else(|e| panic!("{args:?} stdout was not JSON: {e}\n{stdout}"));
    }
}

#[test]
fn a_corrupt_state_file_does_not_break_the_command() {
    let project = tempfile::tempdir().unwrap();
    let cfg = tempfile::tempdir().unwrap();
    let state = cfg.path().join("bb/skills.json");
    std::fs::create_dir_all(state.parent().unwrap()).unwrap();
    std::fs::write(&state, "{not json").unwrap();

    bb(project.path(), cfg.path())
        .args(["skill", "status"])
        .assert()
        .success();
}

/// `status` and `uninstall` must be equally honest about a corrupt state file:
/// both read through `load_state`, so both should warn on stderr rather than
/// one going silent about it.
#[test]
fn a_corrupt_state_file_warns_on_uninstall_too() {
    let project = tempfile::tempdir().unwrap();
    let cfg = tempfile::tempdir().unwrap();
    let state = cfg.path().join("bb/skills.json");
    std::fs::create_dir_all(state.parent().unwrap()).unwrap();
    std::fs::write(&state, "{not json").unwrap();

    bb(project.path(), cfg.path())
        .args(["skill", "uninstall"])
        .assert()
        .success()
        .stderr(contains("skills.json"));
}

/// `--global` must act on `HOME`, never on the project directory, on both
/// `install` and `uninstall`.
#[test]
fn global_install_and_uninstall_target_home_not_the_project() {
    let project = tempfile::tempdir().unwrap();
    let cfg = tempfile::tempdir().unwrap();

    bb(project.path(), cfg.path())
        .args(["skill", "install", "--global"])
        .assert()
        .success();

    let global_path = cfg.path().join(".agents/skills/bitbucket-cloud/SKILL.md");
    let project_path = project
        .path()
        .join(".agents/skills/bitbucket-cloud/SKILL.md");
    assert!(
        global_path.is_file(),
        "global install should write under HOME"
    );
    assert!(
        !project_path.exists(),
        "global install must not touch the project directory"
    );

    bb(project.path(), cfg.path())
        .args(["skill", "uninstall", "--global"])
        .assert()
        .success();
    assert!(
        !global_path.exists(),
        "global uninstall should remove the HOME copy"
    );
}

/// A symlinked Claude entry must be removed as a link, not followed into its
/// target. Uninstalling both agents naturally removes the `.agents` copy too
/// (it's tracked in its own right), so this only proves the `.claude` entry
/// actually disappears — the "did removal follow the link into its target"
/// property is covered by the library-level test in `src/skill.rs`, which
/// scopes the uninstall to just the Claude entry. Tolerates the platform
/// falling back to a real file instead of a symlink, same as the Task 2 test.
/// `install --agent claude` end to end, including a pre-existing hand-made
/// symlink at the Claude location (what the old README's `ln -s` step told
/// users to create). This is the exact gap that let Critical 2 slip through:
/// `--agent claude` alone was never exercised, so `install` recording
/// `kind: "file"` for a symlink it did not create itself went unnoticed.
#[test]
fn install_agent_claude_over_a_hand_made_symlink_then_uninstall_preserves_agents_copy() {
    let project = tempfile::tempdir().unwrap();
    let cfg = tempfile::tempdir().unwrap();

    std::fs::create_dir_all(project.path().join(".agents/skills/bitbucket-cloud")).unwrap();
    std::fs::write(
        project
            .path()
            .join(".agents/skills/bitbucket-cloud/SKILL.md"),
        std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/.agents/skills/bitbucket-cloud/SKILL.md"
        ))
        .unwrap(),
    )
    .unwrap();

    let claude_dir = project.path().join(".claude/skills/bitbucket-cloud");
    std::fs::create_dir_all(claude_dir.parent().unwrap()).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink("../../.agents/skills/bitbucket-cloud", &claude_dir).unwrap();

    bb(project.path(), cfg.path())
        .args(["skill", "install", "--agent", "claude"])
        .assert()
        .success();

    bb(project.path(), cfg.path())
        .args(["skill", "uninstall"])
        .assert()
        .success();

    let agents_file = project
        .path()
        .join(".agents/skills/bitbucket-cloud/SKILL.md");
    assert!(
        agents_file.is_file(),
        "the .agents copy must survive uninstalling the claude link"
    );
    assert!(
        std::fs::symlink_metadata(&claude_dir).is_err(),
        "no dangling claude symlink should remain — Path::exists() would wrongly \
         report false for a dangling link, so this checks symlink_metadata instead"
    );
}

/// Important 4: the human-readable uninstall messages must distinguish
/// "removed", "refused because modified", and "was already gone" rather than
/// collapsing the latter two into the same `false` boolean.
#[test]
fn uninstall_messages_distinguish_removed_refused_and_absent() {
    let project = tempfile::tempdir().unwrap();
    let cfg = tempfile::tempdir().unwrap();

    bb(project.path(), cfg.path())
        .args(["skill", "install", "--agent", "all"])
        .assert()
        .success();

    let agents_path = project
        .path()
        .join(".agents/skills/bitbucket-cloud/SKILL.md");
    std::fs::write(&agents_path, "# ours\n").unwrap();

    let claude_dir = project.path().join(".claude/skills/bitbucket-cloud");
    // Remove the claude side out from under bb, so its tracked entry is
    // "absent" rather than "removed" or "refused".
    if claude_dir.exists() || std::fs::symlink_metadata(&claude_dir).is_ok() {
        let _ = std::fs::remove_dir_all(&claude_dir);
        let _ = std::fs::remove_file(&claude_dir);
    }

    bb(project.path(), cfg.path())
        .args(["skill", "uninstall"])
        .assert()
        .success()
        .stderr(contains(
            "edited locally — left alone (pass --force to remove)",
        ))
        .stdout(contains("already gone — nothing to remove"));
}

#[test]
fn uninstall_removes_a_symlinked_claude_entry() {
    let project = tempfile::tempdir().unwrap();
    let cfg = tempfile::tempdir().unwrap();

    bb(project.path(), cfg.path())
        .args(["skill", "install", "--agent", "all"])
        .assert()
        .success();

    let claude_dir = project.path().join(".claude/skills/bitbucket-cloud");
    assert!(claude_dir.join("SKILL.md").exists() || claude_dir.exists());

    bb(project.path(), cfg.path())
        .args(["skill", "uninstall"])
        .assert()
        .success();

    assert!(
        !claude_dir.exists() && !claude_dir.join("SKILL.md").exists(),
        "the claude entry (link or file) should be gone"
    );
}

/// The embedded skill and the CLI ship together. If a command exists and the
/// skill does not mention it, an agent will never use it.
#[test]
fn skill_documents_build_status() {
    let text = bb_cli::skill::skill_by_name("bitbucket-cloud")
        .unwrap()
        .content;
    assert!(text.contains("bb pr build"), "skill omits `bb pr build`");
    assert!(
        text.contains("--build-status"),
        "skill omits `--build-status`"
    );
    assert!(
        text.contains("build_state"),
        "skill omits the rollup json field"
    );
}

#[test]
fn install_writes_both_skills() {
    let dir = tempfile::tempdir().unwrap();
    bb_in(dir.path())
        .args(["skill", "install", "--agent", "agents", "--json"])
        .assert()
        .success();
    for name in ["bitbucket-cloud", "bbc-daily-brief"] {
        let path = dir.path().join(format!(".agents/skills/{name}/SKILL.md"));
        assert!(path.is_file(), "{name} was not installed");
    }
}

#[test]
fn skill_flag_installs_only_that_skill() {
    let dir = tempfile::tempdir().unwrap();
    bb_in(dir.path())
        .args([
            "skill",
            "install",
            "--agent",
            "agents",
            "--skill",
            "bbc-daily-brief",
            "--json",
        ])
        .assert()
        .success();
    assert!(dir
        .path()
        .join(".agents/skills/bbc-daily-brief/SKILL.md")
        .is_file());
    assert!(!dir
        .path()
        .join(".agents/skills/bitbucket-cloud/SKILL.md")
        .exists());
}

#[test]
fn an_unknown_skill_name_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    bb_in(dir.path())
        .args(["skill", "install", "--skill", "nope", "--json"])
        .assert()
        .failure();
}

#[test]
fn status_json_names_the_skill_per_row() {
    let dir = tempfile::tempdir().unwrap();
    bb_in(dir.path())
        .args(["skill", "install", "--agent", "agents", "--json"])
        .assert()
        .success();
    let out = bb_in(dir.path())
        .args(["skill", "status", "--json"])
        .assert()
        .success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    let rows: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let names: Vec<String> = rows
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["skill"].as_str().unwrap().to_string())
        .collect();
    assert!(
        names.contains(&"bitbucket-cloud".to_string()),
        "got {names:?}"
    );
    assert!(
        names.contains(&"bbc-daily-brief".to_string()),
        "got {names:?}"
    );
}

#[test]
fn editing_one_skill_does_not_make_the_other_modified() {
    let dir = tempfile::tempdir().unwrap();
    bb_in(dir.path())
        .args(["skill", "install", "--agent", "agents", "--json"])
        .assert()
        .success();
    std::fs::write(
        dir.path().join(".agents/skills/bbc-daily-brief/SKILL.md"),
        "locally edited",
    )
    .unwrap();

    let out = bb_in(dir.path())
        .args(["skill", "status", "--json"])
        .assert()
        .success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    let rows: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    for row in rows.as_array().unwrap() {
        let expected = if row["skill"] == "bbc-daily-brief" {
            "modified"
        } else {
            "current"
        };
        assert_eq!(row["state"], expected, "row {row}");
    }
}

#[test]
fn uninstall_with_skill_removes_only_that_one() {
    let dir = tempfile::tempdir().unwrap();
    bb_in(dir.path())
        .args(["skill", "install", "--agent", "agents", "--json"])
        .assert()
        .success();
    bb_in(dir.path())
        .args(["skill", "uninstall", "--skill", "bbc-daily-brief", "--json"])
        .assert()
        .success();
    assert!(!dir
        .path()
        .join(".agents/skills/bbc-daily-brief/SKILL.md")
        .exists());
    assert!(dir
        .path()
        .join(".agents/skills/bitbucket-cloud/SKILL.md")
        .is_file());
}

#[test]
fn the_brief_skill_states_it_is_invoked_only_on_request() {
    let text = bb_cli::skill::skill_by_name("bbc-daily-brief")
        .unwrap()
        .content;
    assert!(text.contains("Never invoke this skill proactively"));
    assert!(text.contains("bb pr mine"));
    assert!(text.contains("Never resolve a comment thread"));
}

#[test]
fn the_main_skill_documents_the_cross_repository_query() {
    let text = bb_cli::skill::skill_by_name("bitbucket-cloud")
        .unwrap()
        .content;
    assert!(text.contains("bb pr mine"));
    assert!(text.contains("bbc-daily-brief"));
    assert!(text.contains("my_role"));
}

/// The brief is written by the agent *for the user*, so its prose must address
/// them as "you" rather than narrating in the first person. The json field names
/// `my_role`/`my_review_state` are the api's wording and must still be
/// documented, so they are stripped before the search; the frontmatter is too,
/// since its `description` quotes what a user says ("what needs my attention").
#[test]
fn the_brief_skill_addresses_the_user_not_itself() {
    let text = bb_cli::skill::skill_by_name("bbc-daily-brief")
        .unwrap()
        .content;
    let body = text
        .split_once("\n---\n")
        .map(|(_, rest)| rest.to_string())
        .unwrap_or_else(|| text.to_string());
    // `pr mine` is the command's own name, and `my_role`/`my_review_state` are
    // api field names: all three must stay documented, none of them is prose.
    let prose = body
        .replace("my_role", "")
        .replace("my_review_state", "")
        .replace("pr mine", "");
    for needle in [" my ", " mine ", " I ", "My ", "Mine "] {
        assert!(
            !prose.contains(needle),
            "first-person prose `{needle}` must not appear in a brief addressed to the user"
        );
    }
    assert!(
        prose.contains("your") || prose.contains("Your"),
        "the brief speaks to the user as `you`"
    );
}

/// The skill texts are compiled into the published binary, so an example copied
/// out of a real terminal session ships a real workspace, repository, colleague
/// or ticket id to everyone who installs `bb`. Examples must use the placeholder
/// `acme` workspace. This test is deliberately a denylist of the shapes that have
/// slipped in before rather than a clever heuristic: add to it, don't outsmart it.
#[test]
fn no_shipped_skill_text_names_a_real_workspace_or_person() {
    for skill in bb_cli::skill::SKILLS.iter() {
        let lower = skill.content.to_lowercase();
        for needle in ["check24", "mailgpt", "hyein", "afal-", "bitbucket.org/check"] {
            assert!(
                !lower.contains(needle),
                "{} ships `{needle}` — examples must use the placeholder `acme` workspace",
                skill.name
            );
        }
    }
}

#[test]
fn the_brief_skill_carries_the_grouped_output_contract() {
    let text = bb_cli::skill::skill_by_name("bbc-daily-brief")
        .unwrap()
        .content;
    assert!(text.contains("YOU'RE BLOCKING"));
    assert!(text.contains("WAITING ON OTHERS"));
    assert!(
        text.contains("bb pr list -R"),
        "the repo-scoped path must be documented"
    );
    assert!(
        text.contains("my_role"),
        "the json field name is still documented"
    );
}

/// The state file lives under `$XDG_CONFIG_HOME/bb/`, and these tests point both
/// `HOME` and `XDG_CONFIG_HOME` at one tempdir, so look in both shapes.
fn state_file(cfg: &std::path::Path) -> std::path::PathBuf {
    let xdg = cfg.join("bb/skills.json");
    if xdg.exists() {
        return xdg;
    }
    cfg.join(".config/bb/skills.json")
}

/// Rewrites every tracked entry's `version` to an old value, the way an upgraded
/// binary finds a state file written by its predecessor.
fn age_the_state_file(cfg: &std::path::Path) {
    let path = state_file(cfg);
    let raw = std::fs::read_to_string(&path).unwrap();
    let mut entries: Vec<serde_json::Value> = serde_json::from_str(&raw).unwrap();
    assert!(!entries.is_empty(), "nothing tracked to age");
    for e in &mut entries {
        e["version"] = serde_json::Value::String("0.0.1".into());
    }
    std::fs::write(&path, serde_json::to_string_pretty(&entries).unwrap()).unwrap();
}

/// Makes the state file claim that `bb` itself wrote `content` at `path`, which
/// is what distinguishes "the binary now ships newer text" (`Stale`, refreshable)
/// from "someone edited this by hand" (`Modified`, left alone). A test that only
/// overwrites the file gets the second case, not the first.
/// `suffix` is matched against the end of each tracked path rather than compared
/// whole: on macOS a tempdir is handed out as `/var/folders/...` but resolves to
/// `/private/var/folders/...`, so an exact string comparison misses.
fn record_as_written_by_bb(cfg: &std::path::Path, suffix: &str, content: &str) {
    let state = state_file(cfg);
    let raw = std::fs::read_to_string(&state).unwrap();
    let mut entries: Vec<serde_json::Value> = serde_json::from_str(&raw).unwrap();
    let mut found = false;
    for e in &mut entries {
        if e["path"].as_str().is_some_and(|p| p.ends_with(suffix)) {
            e["sha256"] =
                serde_json::Value::String(bb_cli::skill::content_hash(content.as_bytes()));
            found = true;
        }
    }
    assert!(found, "no tracked entry ending in {suffix}");
    std::fs::write(&state, serde_json::to_string_pretty(&entries).unwrap()).unwrap();
}

fn state_versions(cfg: &std::path::Path) -> Vec<String> {
    let raw = std::fs::read_to_string(state_file(cfg)).unwrap();
    let entries: Vec<serde_json::Value> = serde_json::from_str(&raw).unwrap();
    entries
        .iter()
        .map(|e| e["version"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn an_unrelated_command_refreshes_skills_left_behind_by_an_upgrade() {
    let dir = tempfile::tempdir().unwrap();
    bb_in(dir.path())
        .args(["skill", "install", "--agent", "agents", "--json"])
        .assert()
        .success();

    let path = dir.path().join(".agents/skills/bitbucket-cloud/SKILL.md");
    let old = "old text from a previous release";
    std::fs::write(&path, old).unwrap();
    record_as_written_by_bb(dir.path(), ".agents/skills/bitbucket-cloud/SKILL.md", old);
    age_the_state_file(dir.path());

    // A command with nothing to do with skills.
    bb_in(dir.path())
        .args(["completions", "bash"])
        .assert()
        .success();

    let after = std::fs::read_to_string(&path).unwrap();
    assert_ne!(
        after, "old text from a previous release",
        "auto-refresh should have rewritten it"
    );
    assert!(after.starts_with("---"), "and written the real skill text");
    assert!(
        state_versions(dir.path()).iter().all(|v| v != "0.0.1"),
        "the recorded version must move forward so the check stops firing"
    );
}

#[test]
fn auto_refresh_prints_nothing_on_stdout() {
    let dir = tempfile::tempdir().unwrap();
    bb_in(dir.path())
        .args(["skill", "install", "--agent", "agents", "--json"])
        .assert()
        .success();
    let path = dir.path().join(".agents/skills/bitbucket-cloud/SKILL.md");
    std::fs::write(&path, "old text").unwrap();
    record_as_written_by_bb(
        dir.path(),
        ".agents/skills/bitbucket-cloud/SKILL.md",
        "old text",
    );
    age_the_state_file(dir.path());

    let out = bb_in(dir.path())
        .args(["skill", "status", "--json"])
        .assert()
        .success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    serde_json::from_str::<serde_json::Value>(stdout.trim())
        .unwrap_or_else(|e| panic!("stdout must be exactly one json value: {e}\n{stdout}"));
}

#[test]
fn the_opt_out_leaves_a_stale_file_alone() {
    let dir = tempfile::tempdir().unwrap();
    bb_in(dir.path())
        .args(["skill", "install", "--agent", "agents", "--json"])
        .assert()
        .success();
    let path = dir.path().join(".agents/skills/bitbucket-cloud/SKILL.md");
    std::fs::write(&path, "old text").unwrap();
    record_as_written_by_bb(
        dir.path(),
        ".agents/skills/bitbucket-cloud/SKILL.md",
        "old text",
    );
    age_the_state_file(dir.path());

    bb_in(dir.path())
        .env("BB_SKILL_NO_AUTO_REFRESH", "1")
        .args(["completions", "bash"])
        .assert()
        .success();

    assert_eq!(std::fs::read_to_string(&path).unwrap(), "old text");
}

#[test]
fn auto_refresh_never_overwrites_a_local_edit() {
    let dir = tempfile::tempdir().unwrap();
    bb_in(dir.path())
        .args(["skill", "install", "--agent", "agents", "--json"])
        .assert()
        .success();
    let path = dir.path().join(".agents/skills/bbc-daily-brief/SKILL.md");
    std::fs::write(&path, "my own notes").unwrap();
    age_the_state_file(dir.path());

    bb_in(dir.path())
        .args(["completions", "bash"])
        .assert()
        .success();

    assert_eq!(std::fs::read_to_string(&path).unwrap(), "my own notes");
    assert!(
        state_versions(dir.path()).iter().all(|v| v != "0.0.1"),
        "a skipped entry still records the running version"
    );
}

#[test]
fn a_state_entry_under_a_deleted_tree_is_pruned_by_any_command() {
    let dir = tempfile::tempdir().unwrap();
    let gone = dir.path().join("was-a-temp-dir");
    std::fs::create_dir_all(&gone).unwrap();
    bb(gone.as_path(), dir.path())
        .args(["skill", "install", "--agent", "agents", "--json"])
        .assert()
        .success();
    std::fs::remove_dir_all(&gone).unwrap();
    age_the_state_file(dir.path());

    bb_in(dir.path())
        .args(["completions", "bash"])
        .assert()
        .success();

    let out = bb_in(dir.path())
        .args(["skill", "status", "--json"])
        .assert()
        .success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert!(
        !stdout.contains("was-a-temp-dir"),
        "the vanished entry should be gone from status: {stdout}"
    );
    assert!(!gone.exists(), "pruning must not recreate the tree");
}
