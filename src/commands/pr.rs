use crate::api::models::{Commit, DiffStatEntry, PullRequest, ReviewerRef, User};
use crate::api::{repo_path, Client};
use crate::credentials;
use crate::error::{BbError, Result};
use crate::git;
use crate::output::{self, Format};
use crate::repo::{self, RepoSlug};
use serde::Serialize;

pub struct Ctx {
    pub client: Client,
    pub slug: RepoSlug,
    pub format: Format,
}

impl Ctx {
    pub fn new(repo: Option<&str>, format: Format) -> Result<Self> {
        let creds = credentials::load()?;
        let slug = repo::resolve(repo)?;
        let client = Client::from_env(creds)?;
        Ok(Self {
            client,
            slug,
            format,
        })
    }

    pub fn path(&self, suffix: &str) -> String {
        repo_path(&self.slug, suffix)
    }
}

#[derive(Debug, Serialize)]
struct PrRow {
    id: u64,
    title: String,
    author: String,
    source: String,
    destination: String,
    reviewers: Vec<String>,
    approvals: Vec<String>,
    url: String,
}

fn to_row(pr: &PullRequest) -> PrRow {
    PrRow {
        id: pr.id,
        title: pr.title.clone().unwrap_or_default(),
        author: pr.author_name().to_string(),
        source: pr.source_branch().to_string(),
        destination: pr.destination_branch().to_string(),
        reviewers: pr
            .reviewers
            .iter()
            .filter_map(|r| r.display_name.clone())
            .collect(),
        approvals: pr
            .participants
            .iter()
            .filter(|p| p.state.as_deref() == Some("approved"))
            .filter_map(|p| p.user.as_ref().and_then(|u| u.display_name.clone()))
            .collect(),
        url: pr.html_url().to_string(),
    }
}

pub async fn list(ctx: &Ctx, destination: Option<String>, state: String) -> Result<()> {
    let spinner = output::spinner("fetching pull requests");
    let prs: Vec<PullRequest> = ctx
        .client
        .paginate(&ctx.path(&format!(
            "/pullrequests?state={}&pagelen=50",
            urlencoding::encode(&state.to_uppercase())
        )))
        .await?;
    spinner.finish_and_clear();

    let rows: Vec<PrRow> = prs
        .iter()
        .filter(|pr| match destination.as_deref() {
            Some(branch) => pr.destination_branch() == branch,
            None => true,
        })
        .map(to_row)
        .collect();

    match ctx.format {
        Format::Json => output::print_json(&rows)?,
        Format::Human => output::print_table(
            &[
                "ID",
                "TITLE",
                "SOURCE",
                "→",
                "TARGET",
                "AUTHOR",
                "REVIEWERS",
                "APPROVED",
            ],
            rows.iter()
                .map(|r| {
                    vec![
                        r.id.to_string(),
                        r.title.clone(),
                        r.source.clone(),
                        "→".into(),
                        r.destination.clone(),
                        r.author.clone(),
                        r.reviewers.join(", "),
                        r.approvals.join(", "),
                    ]
                })
                .collect(),
        ),
    }

    Ok(())
}

pub async fn diff(ctx: &Ctx, id: u64) -> Result<()> {
    let text = ctx
        .client
        .get_text(&ctx.path(&format!("/pullrequests/{id}/diff")))
        .await?;
    if ctx.format.is_json() {
        output::print_json(&serde_json::json!({ "id": id, "diff": text }))?;
    } else {
        print!("{text}");
    }
    Ok(())
}

pub async fn files(ctx: &Ctx, id: u64) -> Result<()> {
    let entries: Vec<DiffStatEntry> = ctx
        .client
        .paginate(&ctx.path(&format!("/pullrequests/{id}/diffstat?pagelen=100")))
        .await?;

    #[derive(Serialize)]
    struct FileRow {
        status: String,
        path: String,
    }

    let rows: Vec<FileRow> = entries
        .iter()
        .map(|e| FileRow {
            status: e.status.clone().unwrap_or_else(|| "-".into()),
            path: e.path().to_string(),
        })
        .collect();

    match ctx.format {
        Format::Json => output::print_json(&rows)?,
        Format::Human => output::print_table(
            &["STATUS", "PATH"],
            rows.iter()
                .map(|r| vec![r.status.clone(), r.path.clone()])
                .collect(),
        ),
    }
    Ok(())
}

pub async fn commits(ctx: &Ctx, id: u64) -> Result<()> {
    let commits: Vec<Commit> = ctx
        .client
        .paginate(&ctx.path(&format!("/pullrequests/{id}/commits?pagelen=100")))
        .await?;

    #[derive(Serialize)]
    struct CommitRow {
        hash: String,
        summary: String,
    }

    let rows: Vec<CommitRow> = commits
        .iter()
        .map(|c| CommitRow {
            hash: c.hash.clone().unwrap_or_default().chars().take(7).collect(),
            summary: c
                .summary
                .as_ref()
                .and_then(|s| s.raw.clone())
                .unwrap_or_default()
                .lines()
                .next()
                .unwrap_or("")
                .to_string(),
        })
        .collect();

    match ctx.format {
        Format::Json => output::print_json(&rows)?,
        Format::Human => output::print_table(
            &["HASH", "SUMMARY"],
            rows.iter()
                .map(|r| vec![r.hash.clone(), r.summary.clone()])
                .collect(),
        ),
    }
    Ok(())
}

pub async fn request_changes(ctx: &Ctx, id: u64) -> Result<()> {
    ctx.client
        .post_empty(&ctx.path(&format!("/pullrequests/{id}/request-changes")))
        .await?;
    report(
        ctx,
        &format!("changes requested on #{id}"),
        serde_json::json!({ "requested_changes": id }),
    )
}

pub async fn unrequest_changes(ctx: &Ctx, id: u64) -> Result<()> {
    ctx.client
        .delete(&ctx.path(&format!("/pullrequests/{id}/request-changes")))
        .await?;
    report(
        ctx,
        &format!("change request removed from #{id}"),
        serde_json::json!({ "unrequested_changes": id }),
    )
}

pub fn report(ctx: &Ctx, human: &str, json: serde_json::Value) -> Result<()> {
    match ctx.format {
        Format::Json => output::print_json(&json),
        Format::Human => {
            output::success(human);
            Ok(())
        }
    }
}

#[derive(Debug, Default)]
pub struct CreateArgs {
    pub target: String,
    pub source: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
    pub no_default_reviewers: bool,
    pub interactive: bool,
    pub web: bool,
    pub close_source_branch: bool,
}

async fn default_reviewers(ctx: &Ctx) -> Result<Vec<ReviewerRef>> {
    let me: User = ctx.client.get_json("/user").await?;
    let my_uuid = me.uuid.unwrap_or_default();
    let reviewers: Vec<User> = ctx.client.paginate(&ctx.path("/default-reviewers")).await?;
    Ok(reviewers
        .into_iter()
        .filter_map(|r| r.uuid)
        .filter(|uuid| *uuid != my_uuid)
        .map(|uuid| ReviewerRef { uuid })
        .collect())
}

pub async fn create(ctx: &Ctx, args: CreateArgs) -> Result<()> {
    let source = match args.source {
        Some(branch) => branch,
        None => git::current_branch()?,
    };

    let mut seen = std::collections::HashSet::new();
    let targets: Vec<String> = args
        .target
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .filter(|s| seen.insert(s.clone()))
        .collect();
    if targets.is_empty() {
        return Err(BbError::Config("no target branch given".into()));
    }
    if targets.contains(&source) {
        return Err(BbError::Config(format!(
            "source and target are both `{source}`"
        )));
    }

    let mut title = args.title;
    let mut description = args.description;
    if args.interactive {
        if title.is_none() {
            let entered = inquire::Text::new("title:")
                .with_help_message("leave empty for the default")
                .prompt()
                .map_err(|e| BbError::Config(format!("cancelled: {e}")))?;
            title = Some(entered).filter(|t| !t.trim().is_empty());
        }
        if description.is_none() {
            let entered = inquire::Editor::new("description:")
                .prompt()
                .map_err(|e| BbError::Config(format!("cancelled: {e}")))?;
            description = Some(entered).filter(|t| !t.trim().is_empty());
        }
    }

    let reviewers = if args.no_default_reviewers {
        Vec::new()
    } else {
        default_reviewers(ctx).await?
    };

    #[derive(Serialize)]
    struct Created {
        id: u64,
        target: String,
        url: String,
    }

    let mut created = Vec::new();
    for target in targets {
        // NOTE: `title` and `description` are `Option<String>` owned across loop
        // iterations, and `serde_json::json!` moves any value given by value. We
        // borrow `&source`/`&target` and use `.as_deref()` on the options so the
        // macro only ever sees references, leaving the originals intact for the
        // next iteration and for the default-title fallback, success line, and
        // `Created { target, .. }` below (where an owned `target` is genuinely
        // needed, so it is consumed there instead of inside `json!`).
        let default_title = format!("Merge {source} into {target}");
        let body_title = title.as_deref().unwrap_or(&default_title);
        let mut body = serde_json::json!({
            "title": body_title,
            "source": { "branch": { "name": &source } },
            "destination": { "branch": { "name": &target } },
            "reviewers": reviewers,
            "close_source_branch": args.close_source_branch,
        });
        if let Some(text) = description.as_deref() {
            body["description"] = serde_json::Value::String(text.to_string());
        }

        let spinner = output::spinner(&format!("opening {source} \u{2192} {target}"));
        let pr: PullRequest = ctx
            .client
            .post_json(&ctx.path("/pullrequests"), &body)
            .await?;
        spinner.finish_and_clear();

        let url = if pr.html_url() == "-" {
            format!("{}/pull-requests/{}", ctx.slug.browse_url(), pr.id)
        } else {
            pr.html_url().to_string()
        };

        if !ctx.format.is_json() {
            output::success(&format!("#{} {source} \u{2192} {target}", pr.id));
            output::info(&url);
        }
        if args.web {
            let _ = open::that_detached(&url);
        }
        created.push(Created {
            id: pr.id,
            target,
            url,
        });
    }

    if ctx.format.is_json() {
        output::print_json(&created)?;
    }
    Ok(())
}
