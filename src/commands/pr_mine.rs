use crate::api;
use crate::api::models::{
    BuildState, BuildStatus, PullRequest, Repository, ReviewState, ReviewerState, Workspace,
};
use crate::api::Client;
use crate::commands::pr_list::REVIEWER_FIELDS;
use crate::credentials;
use crate::error::Result;
use crate::output::{self, Format};
use crate::repo::RepoSlug;
use crate::users::current_user;
use futures::stream::{self, StreamExt};
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum RoleArg {
    /// Pull requests I opened.
    Author,
    /// Pull requests I am tagged to review.
    Reviewer,
    All,
}

#[derive(Debug)]
pub struct MineArgs {
    pub role: RoleArg,
    pub state: String,
    pub workspace: Option<String>,
    pub repo_limit: usize,
    pub build: bool,
}

/// One pull request, flattened to what a brief needs. `repo` is carried on the
/// row because the rows come from many repositories and nothing else identifies
/// which one a given id belongs to.
#[derive(Debug, Serialize)]
struct MineRow {
    repo: String,
    id: u64,
    title: String,
    url: String,
    /// The api's own value, so `--json` stays faithful to bitbucket.
    state: String,
    draft: bool,
    author: String,
    /// "author", "reviewer" or "both".
    my_role: String,
    /// `None` when I am not a reviewer on this pull request.
    my_review_state: Option<ReviewState>,
    reviewers: Vec<ReviewerState>,
    updated_on: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    build_state: Option<BuildState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    build: Option<Vec<BuildStatus>>,
}

/// The scan result. A fixed shape in both directions: a consumer must not have
/// to handle `pull_requests` changing type when one workspace is unreadable.
#[derive(Debug, Serialize)]
struct MineReport {
    pull_requests: Vec<MineRow>,
    /// Workspaces skipped because the token could not read them.
    partial: Vec<String>,
}

fn to_row(repo: &str, pr: &PullRequest, my_uuid: &str) -> MineRow {
    let reviewers = pr.reviewer_states();
    let my_review_state = reviewers
        .iter()
        .find(|r| r.uuid.as_deref() == Some(my_uuid))
        .map(|r| r.state);
    let i_authored = pr.author.as_ref().and_then(|a| a.uuid.as_deref()) == Some(my_uuid);
    let my_role = match (i_authored, my_review_state.is_some()) {
        (true, true) => "both",
        (true, false) => "author",
        _ => "reviewer",
    };
    MineRow {
        repo: repo.to_string(),
        id: pr.id,
        title: pr.title.clone().unwrap_or_default(),
        url: pr.html_url().to_string(),
        state: pr.state.clone().unwrap_or_else(|| "-".into()),
        draft: pr.draft,
        author: pr.author_name().to_string(),
        my_role: my_role.to_string(),
        my_review_state,
        reviewers,
        updated_on: pr.updated_on.clone(),
        build_state: None,
        build: None,
    }
}

/// `all` is a bb-level convenience, matching `bb pr list`.
fn state_query(state: &str) -> String {
    if state.eq_ignore_ascii_case("all") {
        "OPEN,MERGED,DECLINED,SUPERSEDED".to_string()
    } else {
        state.to_uppercase()
    }
}

/// Pull requests I authored, across every workspace, in one paginated call.
async fn authored(
    client: &Client,
    my_uuid: &str,
    state: &str,
) -> Result<Vec<(String, PullRequest)>> {
    let prs: Vec<PullRequest> = client
        .paginate(&format!(
            "/pullrequests/{}?state={}&pagelen=50",
            urlencoding::encode(my_uuid),
            urlencoding::encode(&state_query(state))
        ))
        .await?;
    Ok(prs.into_iter().map(|pr| (repo_of(&pr), pr)).collect())
}

/// The `workspace/repo` a cross-repository result belongs to, read off the
/// pull request's own html link — the authored endpoint returns pull requests
/// from many repositories and this is the only per-row source of that name.
fn repo_of(pr: &PullRequest) -> String {
    let url = pr.html_url();
    let Some(rest) = url.split("bitbucket.org/").nth(1) else {
        return "-".to_string();
    };
    let mut parts = rest.split('/');
    match (parts.next(), parts.next()) {
        (Some(ws), Some(repo)) if !ws.is_empty() && !repo.is_empty() => format!("{ws}/{repo}"),
        _ => "-".to_string(),
    }
}

/// Same bound as the build-status fan-out: fast on a busy morning, clear of the
/// rate limit.
const MAX_IN_FLIGHT: usize = 8;

/// The workspaces to scan. `--workspace` short-circuits the lookup entirely,
/// which is the cheap path a narrowed brief uses.
async fn workspaces(client: &Client, explicit: Option<&str>) -> Result<Vec<String>> {
    if let Some(slug) = explicit {
        return Ok(vec![slug.to_string()]);
    }
    let found: Vec<Workspace> = client.paginate("/workspaces?pagelen=50").await?;
    Ok(found.into_iter().filter_map(|w| w.slug).collect())
}

/// The `--repo-limit` most recently updated repositories in one workspace.
/// Sorting by recency and capping is the bound on the whole reviewer half: a
/// repository nobody has touched in months cannot hold a review waiting on you.
async fn repositories(client: &Client, workspace: &str, limit: usize) -> Result<Vec<String>> {
    let found: Vec<Repository> = client
        .paginate(&format!(
            "/repositories/{}?role=member&sort=-updated_on&pagelen=50",
            urlencoding::encode(workspace)
        ))
        .await?;
    Ok(found
        .into_iter()
        .filter_map(|r| r.full_name)
        .take(limit)
        .collect())
}

/// Pull requests in one repository where I am a reviewer.
async fn reviewing_in(
    client: &Client,
    repo: &str,
    state: &str,
    my_uuid: &str,
) -> Result<Vec<(String, PullRequest)>> {
    let slug = RepoSlug::parse(repo)?;
    let prs: Vec<PullRequest> = client
        .paginate(&api::repo_path(
            &slug,
            &format!(
                "/pullrequests?state={}&pagelen=50&fields={REVIEWER_FIELDS}",
                urlencoding::encode(&state_query(state))
            ),
        ))
        .await?;
    Ok(prs
        .into_iter()
        .filter(|pr| {
            pr.reviewer_states()
                .iter()
                .any(|r| r.uuid.as_deref() == Some(my_uuid))
        })
        .map(|pr| (repo.to_string(), pr))
        .collect())
}

pub async fn run(format: Format, args: MineArgs) -> Result<()> {
    let creds = credentials::load()?;
    let client = Client::from_env(creds)?;

    let me = current_user(&client).await?;
    let my_uuid = me.uuid.unwrap_or_default();

    let spinner = output::spinner("scanning your pull requests");
    let mut found: Vec<(String, PullRequest)> = Vec::new();
    let mut partial: Vec<String> = Vec::new();

    if args.role != RoleArg::Reviewer {
        found.extend(authored(&client, &my_uuid, &args.state).await?);
    }

    if args.role != RoleArg::Author {
        for workspace in workspaces(&client, args.workspace.as_deref()).await? {
            // A token without scope on one workspace must not sink the whole
            // scan; the slug is reported instead, so a brief built from a
            // partial view can say so.
            let repos = match repositories(&client, &workspace, args.repo_limit).await {
                Ok(repos) => repos,
                Err(_) => {
                    partial.push(workspace);
                    continue;
                }
            };
            let batches: Vec<Vec<(String, PullRequest)>> = stream::iter(repos.iter())
                .map(|repo| reviewing_in(&client, repo, &args.state, &my_uuid))
                .buffer_unordered(MAX_IN_FLIGHT)
                .collect::<Vec<_>>()
                .await
                .into_iter()
                .collect::<Result<Vec<_>>>()?;
            for batch in batches {
                found.extend(batch);
            }
        }
    }
    spinner.finish_and_clear();

    let mut rows: Vec<MineRow> = Vec::new();
    for (repo, pr) in &found {
        if rows.iter().any(|r| r.repo == *repo && r.id == pr.id) {
            continue;
        }
        rows.push(to_row(repo, pr, &my_uuid));
    }

    if args.build {
        attach_builds(&client, &mut rows).await?;
    }

    render(format, rows, partial, args.build)
}

/// One statuses fetch per row, grouped by repository so each group reuses one
/// slug. Runs after the merge and dedupe, never before: a duplicated row must
/// not cost a second request.
async fn attach_builds(client: &Client, rows: &mut [MineRow]) -> Result<()> {
    let mut repos: Vec<String> = rows.iter().map(|r| r.repo.clone()).collect();
    repos.sort();
    repos.dedup();
    for repo in repos {
        let Ok(slug) = RepoSlug::parse(&repo) else {
            continue;
        };
        let ids: Vec<u64> = rows
            .iter()
            .filter(|r| r.repo == repo)
            .map(|r| r.id)
            .collect();
        let mut statuses = crate::commands::pr_build::statuses_for(client, &slug, &ids).await?;
        for row in rows.iter_mut().filter(|r| r.repo == repo) {
            let found = statuses.remove(&row.id).unwrap_or_default();
            row.build_state = Some(BuildState::rollup(&found));
            row.build = Some(found);
        }
    }
    Ok(())
}

fn render(format: Format, rows: Vec<MineRow>, partial: Vec<String>, build: bool) -> Result<()> {
    match format {
        Format::Json => {
            let report = MineReport {
                pull_requests: rows,
                partial,
            };
            output::print_json(&report)?;
        }
        Format::Human => {
            if !partial.is_empty() {
                output::warn(&format!(
                    "could not read {} — the scan is incomplete",
                    partial.join(", ")
                ));
            }
            let mut headers: Vec<&str> = vec!["REPO", "ID", "TITLE", "STATE"];
            if build {
                headers.push("BUILD");
            }
            headers.extend(["ROLE", "MINE", "UPDATED"]);
            output::print_table(
                &headers,
                rows.iter()
                    .map(|r| {
                        let mut cells = vec![
                            r.repo.clone(),
                            r.id.to_string(),
                            r.title.clone(),
                            r.state.clone(),
                        ];
                        if build {
                            let state = r.build_state.unwrap_or(BuildState::None);
                            cells
                                .push(output::colored_cell(state.label(), output::tone_for(state)));
                        }
                        cells.extend([
                            r.my_role.clone(),
                            r.my_review_state
                                .map(|s| s.as_str().to_string())
                                .unwrap_or_else(|| "-".into()),
                            r.updated_on
                                .as_deref()
                                .map(output::relative_time)
                                .unwrap_or_else(|| "-".into()),
                        ]);
                        cells
                    })
                    .collect(),
            );
        }
    }
    Ok(())
}
