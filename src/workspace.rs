//! Which Bitbucket workspace a command acts on.
//!
//! There is no api call left that discovers a user's workspaces —
//! `GET /workspaces`, `GET /user/permissions/workspaces` and
//! `GET /user/permissions/repositories` were all removed by Atlassian under
//! CHANGE-2770 and now return 410 — so this resolves entirely from local input.

use crate::api::models::Project;
use crate::api::{workspace_path, Client};
use crate::credentials;
use crate::error::{BbError, Result};
use crate::output::Format;
use crate::repo;

/// Splits a comma-separated `--workspace`/`BB_WORKSPACE` value into slugs:
/// trims whitespace, drops empty segments, and deduplicates while preserving
/// order.
pub fn parse_list(raw: &str) -> Vec<String> {
    let mut out = Vec::new();
    for part in raw.split(',') {
        let slug = part.trim();
        if slug.is_empty() {
            continue;
        }
        if !out.iter().any(|s: &String| s == slug) {
            out.push(slug.to_string());
        }
    }
    out
}

/// The workspaces to act on, in precedence order:
///
/// 1. `--workspace` (comma-separated).
/// 2. `BB_WORKSPACE` (same syntax).
/// 3. The workspace of the git remote in the current checkout, tried rather
///    than required.
/// 4. None of the above: a config error naming both `--workspace` and
///    `BB_WORKSPACE`, rather than silently acting on nothing.
pub fn resolve_list(explicit: Option<&str>) -> Result<Vec<String>> {
    if let Some(raw) = explicit {
        let slugs = parse_list(raw);
        if !slugs.is_empty() {
            return Ok(slugs);
        }
    }
    if let Ok(raw) = std::env::var("BB_WORKSPACE") {
        let slugs = parse_list(&raw);
        if !slugs.is_empty() {
            return Ok(slugs);
        }
    }
    if let Ok(slug) = repo::resolve(None) {
        return Ok(vec![slug.workspace]);
    }
    Err(BbError::Config(
        "no workspace — pass --workspace, set BB_WORKSPACE, or run inside a bitbucket checkout"
            .into(),
    ))
}

/// The single workspace to act on. Commands that operate on exactly one
/// workspace take the first slug of `resolve_list`, so `--workspace a,b` is a
/// harmless superset rather than a second syntax to learn.
///
/// No empty-list guard here: `resolve_list` only ever returns a non-empty
/// `Vec` or an `Err`, never `Ok(vec![])`, so there is nothing for one to
/// catch. Do not "restore" it.
pub fn resolve_one(explicit: Option<&str>) -> Result<String> {
    let mut slugs = resolve_list(explicit)?;
    Ok(slugs.remove(0))
}

/// The per-command context for workspace-scoped commands.
///
/// Deliberately a separate type from `Ctx` rather than `Ctx` with an
/// `Option<RepoSlug>`: a repository that does not exist yet has no slug, and
/// every existing consumer of `Ctx` would otherwise have to handle a `None`
/// that cannot occur for it.
pub struct WorkspaceCtx {
    pub client: Client,
    pub workspace: String,
    pub format: Format,
}

impl WorkspaceCtx {
    pub fn new(workspace: Option<&str>, format: Format) -> Result<Self> {
        let creds = credentials::load()?;
        let workspace = resolve_one(workspace)?;
        let client = Client::from_env(creds)?;
        Ok(Self {
            client,
            workspace,
            format,
        })
    }

    /// `/repositories/{workspace}{suffix}`, percent-encoded exactly once.
    pub fn repos_path(&self, suffix: &str) -> String {
        crate::api::workspace_repos_path(&self.workspace, suffix)
    }

    /// `/workspaces/{workspace}/projects{suffix}`
    pub fn projects_path(&self, suffix: &str) -> String {
        workspace_path(&self.workspace, &format!("/projects{suffix}"))
    }
}

/// Every project in the workspace the token can see.
///
/// One fetch serves both `bb project list` and `bb repo create`'s picker, so
/// the endpoint is written and tested once.
pub async fn projects(ctx: &WorkspaceCtx) -> Result<Vec<Project>> {
    ctx.client
        .paginate(&ctx.projects_path("?pagelen=100"))
        .await
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn splits_trims_and_dedupes() {
        assert_eq!(parse_list(" acme , , acme ,other"), vec!["acme", "other"]);
    }

    #[test]
    fn empty_input_yields_no_slugs() {
        assert!(parse_list("  ,  ").is_empty());
    }

    #[test]
    fn resolve_one_prefers_the_explicit_value() {
        // An explicit value must win without consulting the environment or git,
        // which is what makes the resolver testable at all.
        assert_eq!(resolve_one(Some("acme")).unwrap(), "acme");
    }

    #[test]
    fn resolve_one_takes_the_first_of_a_list() {
        assert_eq!(resolve_one(Some("first,second")).unwrap(), "first");
    }

    #[test]
    fn resolve_one_rejects_a_blank_explicit_value_by_falling_through() {
        // A `--workspace ""` must not resolve to an empty slug, which would
        // build the url `/repositories/`. Falling through is correct; what must
        // never happen is `Ok("")`.
        assert_ne!(resolve_one(Some("   ")).ok().as_deref(), Some(""));
    }
}
