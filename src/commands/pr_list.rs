use crate::api::models::{BuildState, BuildStatus, PullRequest, ReviewState, ReviewerState};
use crate::commands::pr::Ctx;
use crate::commands::pr_build;
use crate::error::Result;
use crate::output::{self, Format};
use crate::users::{current_user, resolve_user};
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum ReviewStateArg {
    Approved,
    ChangesRequested,
    Pending,
}

impl ReviewStateArg {
    fn as_state(self) -> ReviewState {
        match self {
            Self::Approved => ReviewState::Approved,
            Self::ChangesRequested => ReviewState::ChangesRequested,
            Self::Pending => ReviewState::Pending,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum BuildStateArg {
    Successful,
    Failed,
    Inprogress,
    Stopped,
    /// No check ever reported on the pull request.
    None,
}

impl BuildStateArg {
    fn as_state(self) -> BuildState {
        match self {
            Self::Successful => BuildState::Successful,
            Self::Failed => BuildState::Failed,
            Self::Inprogress => BuildState::InProgress,
            Self::Stopped => BuildState::Stopped,
            Self::None => BuildState::None,
        }
    }
}

#[derive(Debug)]
pub struct ListArgs {
    pub destination: Option<String>,
    pub state: String,
    pub reviewer: Option<String>,
    pub author: Option<String>,
    pub review_state: Option<ReviewStateArg>,
    pub needs_my_review: bool,
    /// Show the build column. Costs one extra request per pull request.
    pub build: bool,
    pub build_status: Option<BuildStateArg>,
}

/// Bitbucket's paginated pull-request endpoint returns a reduced object that omits
/// reviewers, participants and the draft flag. They come back only when asked for
/// explicitly with a partial-response parameter.
///
/// The `+` must arrive url-encoded as `%2B`: a bare `+` in a query string decodes
/// as a space and bitbucket then ignores the whole parameter, which is exactly the
/// silent failure this feature exists to fix.
pub(crate) const REVIEWER_FIELDS: &str =
    "%2Bvalues.reviewers,%2Bvalues.participants,%2Bvalues.draft";

const ALL_STATES: &str = "OPEN,MERGED,DECLINED,SUPERSEDED";

#[derive(Debug, Serialize)]
struct PrRow {
    id: u64,
    title: String,
    /// The api's own value, so `--json` stays faithful to bitbucket.
    state: String,
    draft: bool,
    /// The one word the table shows, folding `draft` into `state`. Carried on the
    /// row rather than recomputed at render time, because filtering means the rows
    /// and the fetched pull requests are no longer index-aligned.
    #[serde(skip)]
    display_state: String,
    author: String,
    source: String,
    destination: String,
    reviewers: Vec<ReviewerState>,
    url: String,
    /// Absent unless the build column was asked for, so today's `--json` shape
    /// is unchanged for existing callers.
    #[serde(skip_serializing_if = "Option::is_none")]
    build_state: Option<BuildState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    build: Option<Vec<BuildStatus>>,
}

fn to_row(pr: &PullRequest) -> PrRow {
    PrRow {
        id: pr.id,
        title: pr.title.clone().unwrap_or_default(),
        state: pr.state.clone().unwrap_or_else(|| "-".into()),
        draft: pr.draft,
        display_state: pr.display_state(),
        author: pr.author_name().to_string(),
        source: pr.source_branch().to_string(),
        destination: pr.destination_branch().to_string(),
        reviewers: pr.reviewer_states(),
        url: pr.html_url().to_string(),
        build_state: None,
        build: None,
    }
}

fn reviewer_cell(reviewers: &[ReviewerState]) -> String {
    reviewers
        .iter()
        .map(|r| format!("{} {}", r.name, r.state.mark()))
        .collect::<Vec<_>>()
        .join(", ")
}

/// `all` and `draft` are bb-level conveniences, not bitbucket states. `draft` is a
/// boolean on an OPEN pull request, so it asks for OPEN and filters afterwards.
fn state_query(state: &str) -> String {
    if state.eq_ignore_ascii_case("all") {
        ALL_STATES.to_string()
    } else if state.eq_ignore_ascii_case("draft") {
        "OPEN".to_string()
    } else {
        state.to_uppercase()
    }
}

/// The uuid of whoever the token belongs to, fetched at most once per invocation
/// and only when a filter actually needs it.
async fn my_uuid(ctx: &Ctx) -> Result<Option<String>> {
    Ok(current_user(&ctx.client).await?.uuid)
}

fn my_review_state(pr: &PullRequest, my_uuid: Option<&str>) -> Option<ReviewState> {
    let me = my_uuid?;
    pr.reviewer_states()
        .into_iter()
        .find(|r| r.uuid.as_deref() == Some(me))
        .map(|r| r.state)
}

pub async fn list(ctx: &Ctx, args: ListArgs) -> Result<()> {
    // Resolve everything the filters need before fetching, so a bad name fails
    // fast instead of after a paginated download.
    let reviewer_uuid = match args.reviewer.as_deref() {
        Some(name) => resolve_user(&ctx.client, &ctx.slug, name, &[]).await?.uuid,
        None => None,
    };

    // `GET /user` must happen at most once per invocation, so every flag that
    // needs "who am I" (`--author @me`, `--needs-my-review`, `--review-state`)
    // shares this single fetch instead of each fetching it independently.
    let author_is_me = args.author.as_deref() == Some("@me");
    let me = if author_is_me || args.needs_my_review || args.review_state.is_some() {
        my_uuid(ctx).await?
    } else {
        None
    };

    let author_uuid = match args.author.as_deref() {
        Some("@me") => me.clone(),
        Some(name) => resolve_user(&ctx.client, &ctx.slug, name, &[]).await?.uuid,
        None => None,
    };

    let want_draft = args.state.eq_ignore_ascii_case("draft");

    let spinner = output::spinner("fetching pull requests");
    let prs: Vec<PullRequest> = ctx
        .client
        .paginate(&ctx.path(&format!(
            "/pullrequests?state={}&pagelen=50&fields={REVIEWER_FIELDS}",
            urlencoding::encode(&state_query(&args.state))
        )))
        .await?;
    spinner.finish_and_clear();

    let kept: Vec<&PullRequest> = prs
        .iter()
        .filter(|pr| match args.destination.as_deref() {
            Some(branch) => pr.destination_branch() == branch,
            None => true,
        })
        .filter(|pr| !want_draft || pr.draft)
        .filter(|pr| match reviewer_uuid.as_deref() {
            Some(uuid) => pr
                .reviewer_states()
                .iter()
                .any(|r| r.uuid.as_deref() == Some(uuid)),
            None => true,
        })
        .filter(|pr| match author_uuid.as_deref() {
            Some(uuid) => pr.author.as_ref().and_then(|a| a.uuid.as_deref()) == Some(uuid),
            None => true,
        })
        .filter(|pr| match args.review_state {
            Some(wanted) => my_review_state(pr, me.as_deref()) == Some(wanted.as_state()),
            None => true,
        })
        .filter(|pr| {
            if !args.needs_my_review {
                return true;
            }
            // I am a reviewer and I have not approved.
            matches!(
                my_review_state(pr, me.as_deref()),
                Some(ReviewState::ChangesRequested) | Some(ReviewState::Pending)
            )
        })
        .collect();

    let mut rows: Vec<PrRow> = kept.iter().map(|pr| to_row(pr)).collect();

    // Build status is a per-pull-request endpoint, so this is the one place the
    // command can cost more than one request. Fetching after the filters keeps
    // `--author @me --build` at one request per surviving row, not per row in
    // the repository.
    let want_build = args.build || args.build_status.is_some();
    if want_build {
        let ids: Vec<u64> = rows.iter().map(|r| r.id).collect();
        let spinner = output::spinner("fetching build statuses");
        let mut statuses = pr_build::statuses_for(&ctx.client, &ctx.slug, &ids).await?;
        spinner.finish_and_clear();
        for row in &mut rows {
            let found = statuses.remove(&row.id).unwrap_or_default();
            row.build_state = Some(BuildState::rollup(&found));
            row.build = Some(found);
        }
        if let Some(wanted) = args.build_status {
            rows.retain(|r| r.build_state == Some(wanted.as_state()));
        }
    }

    render(ctx, &rows, want_build)
}

fn build_cell(state: Option<BuildState>) -> String {
    let state = state.unwrap_or(BuildState::None);
    output::colored_cell(state.label(), output::tone_for(state))
}

fn render(ctx: &Ctx, rows: &[PrRow], build: bool) -> Result<()> {
    match ctx.format {
        Format::Json => output::print_json(&rows)?,
        Format::Human => {
            let mut headers: Vec<&str> = vec!["ID", "TITLE", "STATE"];
            if build {
                headers.push("BUILD");
            }
            headers.extend(["SOURCE", "→", "TARGET", "AUTHOR", "REVIEWERS"]);
            output::print_table(
                &headers,
                rows.iter()
                    .map(|r| {
                        let mut cells =
                            vec![r.id.to_string(), r.title.clone(), r.display_state.clone()];
                        if build {
                            cells.push(build_cell(r.build_state));
                        }
                        cells.extend([
                            r.source.clone(),
                            "→".into(),
                            r.destination.clone(),
                            r.author.clone(),
                            reviewer_cell(&r.reviewers),
                        ]);
                        cells
                    })
                    .collect(),
            )
        }
    }
    Ok(())
}
