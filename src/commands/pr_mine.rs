use crate::api;
use crate::api::models::{
    BuildState, BuildStatus, PullRequest, Repository, ReviewState, ReviewerState,
};
use crate::api::Client;
use crate::commands::pr_list::{state_query, REVIEWER_FIELDS};
use crate::credentials;
use crate::error::{BbError, Result};
use crate::output::{self, Format};
use crate::repo::{self, RepoSlug};
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

/// Which half of the scan a `(repo, pull request)` pair came from, tracked
/// alongside it so the dedupe merge in `run` can decide `my_role` from where
/// the row was actually found rather than from re-deriving it off the pull
/// request's own fields a second time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Origin {
    Authored,
    Reviewing,
}

impl Origin {
    fn as_role(self) -> &'static str {
        match self {
            Origin::Authored => "author",
            Origin::Reviewing => "reviewer",
        }
    }
}

/// The browser url for a pull request. Bitbucket normally supplies it in
/// `links.html.href`, and that value is preferred; when it is absent
/// `html_url()` yields `"-"`, which would reach a consumer as a dead link — and
/// the daily brief renders this field as a clickable link, so a placeholder is
/// worse than a reconstruction. The web url is stable and derivable from the
/// repository and the id, so derive it.
fn browse_url(repo: &str, pr: &PullRequest) -> String {
    let from_api = pr.html_url();
    if from_api != "-" {
        return from_api.to_string();
    }
    format!("https://bitbucket.org/{repo}/pull-requests/{}", pr.id)
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
        url: browse_url(repo, pr),
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

/// Pull requests I authored, in one workspace, in one paginated call.
///
/// `GET /pullrequests/{uuid}` — the cross-workspace form of this endpoint —
/// was removed by Atlassian on 2025-02-20 and now returns 404. The supported
/// replacement is workspace-scoped: `GET /workspaces/{workspace}/pullrequests/{uuid}`,
/// which takes the same `state` (repeatable) and pagination parameters, so the
/// caller now loops this over every workspace instead of making one
/// cross-workspace call.
///
/// The endpoint returns the same reduced object as the paginated
/// per-repository endpoint — see `REVIEWER_FIELDS`'s doc comment — so this
/// must ask for the same partial-response fields the reviewer half does, or a
/// row's `draft`, `reviewers` and `my_review_state` all come back wrong
/// instead of merely missing.
async fn authored(
    client: &Client,
    workspace: &str,
    my_uuid: &str,
    state: &str,
) -> Result<Vec<(String, PullRequest)>> {
    let prs: Vec<PullRequest> = client
        .paginate(&format!(
            "/workspaces/{}/pullrequests/{}?state={}&pagelen=50&fields={REVIEWER_FIELDS}",
            urlencoding::encode(workspace),
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

/// Splits a comma-separated `--workspace`/`BB_WORKSPACE` value into slugs:
/// trims whitespace, drops empty segments, and deduplicates while preserving
/// order.
fn parse_workspace_list(raw: &str) -> Vec<String> {
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

/// The workspaces to scan, in precedence order:
///
/// 1. `--workspace` (comma-separated).
/// 2. `BB_WORKSPACE` (same syntax).
/// 3. The workspace of the git remote in the current checkout, resolved the
///    same way every other command resolves a repository — but tried rather
///    than required, since `pr mine` must work outside a checkout as long as
///    one of the first two sources is given.
/// 4. Neither present and no checkout: a config error naming both `--workspace`
///    and `BB_WORKSPACE`, rather than silently scanning nothing.
///
/// There is no api call left that discovers a user's workspaces —
/// `GET /workspaces`, `GET /user/permissions/workspaces` and
/// `GET /user/permissions/repositories` were all removed by Atlassian under
/// CHANGE-2770 and now return 410 — so this resolves entirely from local
/// input.
fn resolve_workspaces(explicit: Option<&str>) -> Result<Vec<String>> {
    if let Some(raw) = explicit {
        let slugs = parse_workspace_list(raw);
        if !slugs.is_empty() {
            return Ok(slugs);
        }
    }
    if let Ok(raw) = std::env::var("BB_WORKSPACE") {
        let slugs = parse_workspace_list(&raw);
        if !slugs.is_empty() {
            return Ok(slugs);
        }
    }
    if let Ok(slug) = repo::resolve(None) {
        return Ok(vec![slug.workspace]);
    }
    Err(BbError::Config(
        "no workspace to scan — pass --workspace <slug>[,<slug>...], set BB_WORKSPACE, \
         or run inside a bitbucket checkout"
            .into(),
    ))
}

/// The `--repo-limit` most recently updated repositories in one workspace.
/// Sorting by recency and capping is the bound on the whole reviewer half: a
/// repository nobody has touched in months cannot hold a review waiting on you.
///
/// `--repo-limit 0` means scan nothing, and does not even ask — a zero-sized
/// request is a request purely to discard. Otherwise this fetches exactly one
/// page, sized to the limit (capped at bitbucket's own page-size ceiling of
/// 100): `sort=-updated_on` already puts the wanted repositories on page one,
/// so following `next` here would only pay for rows that `.take(limit)` was
/// always going to throw away.
async fn repositories(client: &Client, workspace: &str, limit: usize) -> Result<Vec<String>> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    let pagelen = limit.min(100);
    // `role=member` was removed by Atlassian on 2026-04-14 under CHANGE-2770
    // and now returns 410; the unfiltered workspace listing is the supported
    // replacement.
    let page: api::Page<Repository> = client
        .get_json(&format!(
            "/repositories/{}?sort=-updated_on&pagelen={pagelen}",
            urlencoding::encode(workspace)
        ))
        .await?;
    Ok(page
        .values
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
    // `draft` is a boolean on an individual pull request, not a state the api
    // will filter on, and there is no per-row `draft` flag here to filter on
    // afterwards the way `pr list --state draft` does — a cross-workspace row
    // needs no such degradation, so this is rejected rather than silently
    // asking bitbucket for an invalid `DRAFT` state.
    if args.state.eq_ignore_ascii_case("draft") {
        return Err(BbError::Config(
            "pr mine does not support --state draft — use `bb pr list --state draft` \
             inside the repository, or `--role author` and check the `draft` field"
                .into(),
        ));
    }

    let workspaces = resolve_workspaces(args.workspace.as_deref())?;

    let creds = credentials::load()?;
    let client = Client::from_env(creds)?;

    let me = current_user(&client).await?;
    let my_uuid = me.uuid.ok_or_else(|| {
        BbError::Config(
            "your bitbucket account has no uuid — cannot identify your pull requests".into(),
        )
    })?;

    let spinner = output::spinner("scanning your pull requests");
    let mut found: Vec<(String, PullRequest, Origin)> = Vec::new();
    let mut partial: Vec<String> = Vec::new();

    // The authored half moved to a workspace-scoped endpoint (see `authored`'s
    // doc comment), so it now needs the workspace list too — for every role,
    // not only the reviewer half. `workspaces` is resolved once above, before
    // any request, per `resolve_workspaces`'s precedence order.
    for workspace in workspaces {
        if args.role != RoleArg::Reviewer {
            match authored(&client, &workspace, &my_uuid, &args.state).await {
                Ok(prs) => found.extend(
                    prs.into_iter()
                        .map(|(repo, pr)| (repo, pr, Origin::Authored)),
                ),
                Err(crate::error::BbError::Api { status: 403, .. }) => {
                    partial.push(workspace.clone());
                }
                Err(e) => return Err(e),
            }
        }

        if args.role != RoleArg::Author {
            // A 403 means the token has no scope on this workspace, which is
            // expected on a shared account and must not sink the whole scan —
            // the slug is reported instead, so a brief built from a partial
            // view can say so. Anything else (401, 429, a network failure, a
            // malformed response) is a real failure and must propagate.
            let repos = match repositories(&client, &workspace, args.repo_limit).await {
                Ok(repos) => repos,
                Err(crate::error::BbError::Api { status: 403, .. }) => {
                    if !partial.contains(&workspace) {
                        partial.push(workspace);
                    }
                    continue;
                }
                Err(e) => return Err(e),
            };
            let batches: Vec<Vec<(String, PullRequest)>> = stream::iter(repos.iter())
                .map(|repo| reviewing_in(&client, repo, &args.state, &my_uuid))
                .buffer_unordered(MAX_IN_FLIGHT)
                .collect::<Vec<_>>()
                .await
                .into_iter()
                .collect::<Result<Vec<_>>>()?;
            for batch in batches {
                found.extend(
                    batch
                        .into_iter()
                        .map(|(repo, pr)| (repo, pr, Origin::Reviewing)),
                );
            }
        }
    }
    spinner.finish_and_clear();

    // Which half a pull request was found in is tracked explicitly rather than
    // re-derived from `to_row`'s own reading of the pull request's fields —
    // that way a pull request found by both halves ends as one row marked
    // "both" regardless of whether the api's own reviewer/author fields agree,
    // instead of silently depending on the first-seen half having the richer
    // (or even correct) data.
    let mut rows: Vec<MineRow> = Vec::new();
    for (repo, pr, origin) in &found {
        let this_role = origin.as_role();
        match rows.iter_mut().find(|r| r.repo == *repo && r.id == pr.id) {
            Some(existing) => {
                if existing.my_role != this_role {
                    existing.my_role = "both".to_string();
                }
            }
            None => rows.push(to_row(repo, pr, &my_uuid)),
        }
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
            // A link-less row (`repo == "-"`) still owes every sibling row the
            // same shape when `--build` was asked for — both fields carry
            // `skip_serializing_if`, so leaving them `None` here would make
            // this row's JSON shape differ from every other row's for no
            // reason a consumer could name.
            for row in rows.iter_mut().filter(|r| r.repo == repo) {
                row.build_state = Some(BuildState::None);
                row.build = Some(Vec::new());
            }
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

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn pr_from(json: &str) -> PullRequest {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn browse_url_prefers_the_api_link() {
        let pr = pr_from(
            r#"{"id":42,"links":{"html":{"href":"https://bitbucket.org/acme/api/pull-requests/42"}}}"#,
        );
        assert_eq!(
            browse_url("acme/api", &pr),
            "https://bitbucket.org/acme/api/pull-requests/42"
        );
    }

    #[test]
    fn browse_url_is_derived_when_the_api_omits_the_link() {
        // `html_url()` yields "-" here, which would reach the daily brief as a
        // dead markdown link.
        let pr = pr_from(r#"{"id":42}"#);
        assert_eq!(
            browse_url("acme/api", &pr),
            "https://bitbucket.org/acme/api/pull-requests/42"
        );
    }
}
