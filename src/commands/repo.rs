use crate::api::models::Project;
use crate::api::{workspace_path, Client};
use crate::credentials;
use crate::error::Result;
use crate::output::Format;
use crate::workspace;

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
        let workspace = workspace::resolve_one(workspace)?;
        let client = Client::from_env(creds)?;
        Ok(Self {
            client,
            workspace,
            format,
        })
    }

    /// `/repositories/{workspace}{suffix}`, percent-encoded exactly once.
    pub fn repos_path(&self, suffix: &str) -> String {
        format!(
            "/repositories/{}{}",
            urlencoding::encode(&self.workspace),
            suffix
        )
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
