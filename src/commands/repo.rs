use crate::api::models::Repository;
use crate::error::{BbError, Result};
use crate::output::{self, Format};
use crate::workspace::{projects, WorkspaceCtx};
use serde::Serialize;

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

/// The creation request body.
///
/// Only three fields, and two of them optional. `scm`, `fork_policy`,
/// `mainbranch`, `has_wiki` and `has_issues` are deliberately absent so
/// bitbucket and the workspace's own settings decide them.
#[derive(Debug, Serialize)]
struct CreateBody {
    /// Sent always. Omitting it does not reliably produce a private
    /// repository — the effective default depends on workspace configuration,
    /// and getting it wrong publishes source code to the internet.
    is_private: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    project: Option<ProjectKey>,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
}

#[derive(Debug, Serialize)]
struct ProjectKey {
    key: String,
}

/// The project the new repository goes into.
///
/// The lookup runs only on the picker path, so `--project` costs no extra
/// request — the same rule `pr resolve` and the request-changes gate follow: a
/// caller that already has its answer must not pay for a prompt it will never
/// see.
async fn resolve_project(ctx: &WorkspaceCtx, explicit: Option<String>) -> Result<String> {
    if let Some(key) = explicit {
        return Ok(key);
    }
    if !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
        return Err(BbError::Config(
            "a project is required — pass --project KEY".into(),
        ));
    }

    let spinner = output::spinner("fetching projects");
    let projects = projects(ctx).await?;
    spinner.finish_and_clear();

    let choices: Vec<String> = projects
        .iter()
        .filter(|p| p.key.is_some())
        .map(|p| format!("{} — {}", p.key_or_dash(), p.name_or_dash()))
        .collect();
    if choices.is_empty() {
        return Err(BbError::Config(format!(
            "workspace {} has no projects you can see — pass --project KEY",
            ctx.workspace
        )));
    }

    // inquire writes to stderr, so `--json` stdout stays pure.
    let picked = inquire::Select::new("which project?", choices)
        .prompt()
        .map_err(|e| BbError::Config(format!("no project chosen: {e}")))?;
    Ok(picked
        .split_once(" — ")
        .map(|(key, _)| key.to_string())
        .unwrap_or(picked))
}

pub async fn create(
    ctx: &WorkspaceCtx,
    name: String,
    project: Option<String>,
    description: Option<String>,
    public: bool,
) -> Result<()> {
    let key = resolve_project(ctx, project).await?;

    let body = CreateBody {
        is_private: !public,
        project: Some(ProjectKey { key }),
        description,
    };

    // `name` is sent verbatim as the slug: bitbucket normalises it and derives
    // the display name. A local guess that disagrees with the server's would
    // produce a url that 404s.
    let path = format!("/{}", urlencoding::encode(&name));
    let spinner = output::spinner("creating repository");
    let repo: Repository = ctx
        .client
        .post_json(&ctx.repos_path(&path), &body)
        .await
        .inspect_err(|_| spinner.finish_and_clear())?;
    spinner.finish_and_clear();

    match ctx.format {
        Format::Json => output::print_json(&repo)?,
        Format::Human => {
            output::success(&format!(
                "created {} in project {}",
                repo.full_name.as_deref().unwrap_or(repo.display_name()),
                repo.project_key()
            ));
            // A missing convenience url must never fail a create that
            // succeeded, so each line is printed only if the server sent it.
            if let Some(url) = repo.html_url() {
                println!("  {url}");
            }
            if let Some(url) = repo.clone_url() {
                println!("  {url}");
            }
        }
    }
    Ok(())
}
