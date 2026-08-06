use crate::error::{BbError, Result};
use std::path::Path;
use std::process::Command;

/// Runs `git` with an explicit argument vector. No shell is involved, so no
/// argument can be interpreted as a command. Runs in `dir` if given,
/// otherwise in the process's current directory.
fn git_in(dir: Option<&Path>, args: &[&str]) -> Result<String> {
    let mut cmd = Command::new("git");
    cmd.args(args);
    if let Some(dir) = dir {
        cmd.current_dir(dir);
    }
    let output = cmd
        .output()
        .map_err(|e| BbError::Git(format!("cannot run git: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(BbError::Git(if stderr.is_empty() {
            format!("git {} failed", args.join(" "))
        } else {
            stderr
        }));
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn git(args: &[&str]) -> Result<String> {
    git_in(None, args)
}

pub fn current_branch() -> Result<String> {
    let branch = git(&["symbolic-ref", "--short", "HEAD"])?;
    if branch.is_empty() {
        return Err(BbError::Git(
            "detached HEAD — cannot infer source branch".into(),
        ));
    }
    Ok(branch)
}

pub fn remote_url(remote: &str) -> Result<String> {
    let url = git(&["config", "--get", &format!("remote.{remote}.url")])?;
    if url.is_empty() {
        return Err(BbError::Git(format!("remote `{remote}` has no url")));
    }
    Ok(url)
}

/// Remote names as `git remote` lists them, in git's own order.
pub fn remotes() -> Result<Vec<String>> {
    remotes_in(None)
}

fn remotes_in(dir: Option<&Path>) -> Result<Vec<String>> {
    Ok(git_in(dir, &["remote"])?
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect())
}

pub fn in_repo() -> bool {
    git(&["rev-parse", "--git-dir"]).is_ok()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn remotes_is_empty_for_a_repo_with_no_remotes() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();

        git_in(Some(dir), &["init"]).unwrap();

        let names = remotes_in(Some(dir)).unwrap();
        assert!(names.is_empty(), "expected no remotes, got {names:?}");
    }

    #[test]
    fn remotes_lists_configured_remote_names() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();

        git_in(Some(dir), &["init"]).unwrap();
        git_in(
            Some(dir),
            &["remote", "add", "origin", "https://example.com/origin.git"],
        )
        .unwrap();
        git_in(
            Some(dir),
            &[
                "remote",
                "add",
                "bitbucket",
                "https://example.com/bitbucket.git",
            ],
        )
        .unwrap();

        let names = remotes_in(Some(dir)).unwrap();
        assert_eq!(names, vec!["bitbucket".to_string(), "origin".to_string()]);
    }

    #[test]
    fn git_failure_is_reported_not_panicked() {
        // Modern git (2.50+) exits 0 for unknown `rev-parse` flags, treating them
        // as literal output instead of erroring. Use a ref-verification failure,
        // which reliably exits non-zero across git versions, to exercise the
        // BbError::Git error path without panicking.
        let err = git(&[
            "rev-parse",
            "--verify",
            "refs/heads/definitely-not-a-branch-xyz",
        ])
        .unwrap_err();
        assert!(matches!(err, BbError::Git(_)));
    }
}
