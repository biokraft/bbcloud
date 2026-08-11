#![allow(clippy::unwrap_used)] // test code is exempt from the unwrap/expect ban

use assert_cmd::Command;
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
        .stdout(contains("modified"));
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
