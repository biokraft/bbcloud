use crate::api::models::{Project, Repository};
use crate::api::{workspace_path, Client};
use crate::credentials;
use crate::error::Result;
use crate::output::{self, Format};
use crate::workspace;
use serde::Serialize;

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

#[derive(Debug, Serialize)]
struct RepoRow {
    name: String,
    project: String,
    access: String,
    updated: String,
}

pub async fn list(
    ctx: &WorkspaceCtx,
    project: Option<String>,
    name: Option<String>,
    limit: usize,
) -> Result<()> {
    // `q` narrows server-side so a large workspace is not paged through only
    // to be discarded locally. The quotes are part of bitbucket's query
    // grammar, and `urlencoding::encode` covers them along with the `=`.
    let query = match &project {
        Some(key) => format!(
            "?pagelen=100&sort=-updated_on&q={}",
            urlencoding::encode(&format!("project.key=\"{key}\""))
        ),
        None => "?pagelen=100&sort=-updated_on".to_string(),
    };

    let spinner = output::spinner("fetching repositories");
    let repos: Vec<Repository> = ctx.client.paginate(&ctx.repos_path(&query)).await?;
    spinner.finish_and_clear();

    let needle = name.map(|n| n.to_lowercase());
    let rows: Vec<RepoRow> = repos
        .iter()
        .filter(|r| match &needle {
            Some(needle) => r.display_name().to_lowercase().contains(needle),
            None => true,
        })
        .take(limit)
        .map(|r| RepoRow {
            name: r.display_name().to_string(),
            project: r.project_key().to_string(),
            access: r.access().to_string(),
            updated: r
                .updated_on
                .as_deref()
                .map(output::relative_time)
                .unwrap_or_else(|| "-".into()),
        })
        .collect();

    match ctx.format {
        Format::Json => output::print_json(&rows)?,
        Format::Human => output::print_table(
            &["NAME", "PROJECT", "ACCESS", "UPDATED"],
            rows.iter()
                .map(|r| {
                    vec![
                        r.name.clone(),
                        r.project.clone(),
                        r.access.clone(),
                        r.updated.clone(),
                    ]
                })
                .collect(),
        ),
    }
    Ok(())
}
