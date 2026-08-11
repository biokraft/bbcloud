use crate::api::models::{PullRequest, ReviewState, ReviewerState, User};
use crate::commands::pr::Ctx;
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

#[derive(Debug, Default)]
pub struct ListArgs {
    pub destination: Option<String>,
    pub state: String,
    pub reviewer: Option<String>,
    pub author: Option<String>,
    pub review_state: Option<ReviewStateArg>,
    pub needs_my_review: bool,
}

/// Bitbucket's paginated pull-request endpoint returns a reduced object that omits
/// reviewers, participants and the draft flag. They come back only when asked for
/// explicitly with a partial-response parameter.
///
/// The `+` must arrive url-encoded as `%2B`: a bare `+` in a query string decodes
/// as a space and bitbucket then ignores the whole parameter, which is exactly the
/// silent failure this feature exists to fix.
const REVIEWER_FIELDS: &str = "%2Bvalues.reviewers,%2Bvalues.participants,%2Bvalues.draft";

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
    let author_match: Option<User> = match args.author.as_deref() {
        Some("@me") => Some(current_user(&ctx.client).await?),
        Some(name) => Some(resolve_user(&ctx.client, &ctx.slug, name, &[]).await?),
        None => None,
    };
    let me = if args.needs_my_review || args.review_state.is_some() {
        my_uuid(ctx).await?
    } else {
        None
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

    let rows: Vec<PrRow> = prs
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
        .filter(|pr| match &author_match {
            Some(who) => {
                let uuid_match = who.uuid.is_some()
                    && pr.author.as_ref().and_then(|a| a.uuid.as_deref()) == who.uuid.as_deref();
                let name_match = pr.author.as_ref().map(User::name) == Some(who.name());
                uuid_match || name_match
            }
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
        .map(to_row)
        .collect();

    render(ctx, &rows)
}

fn render(ctx: &Ctx, rows: &[PrRow]) -> Result<()> {
    match ctx.format {
        Format::Json => output::print_json(&rows)?,
        Format::Human => output::print_table(
            &[
                "ID",
                "TITLE",
                "STATE",
                "SOURCE",
                "→",
                "TARGET",
                "AUTHOR",
                "REVIEWERS",
            ],
            rows.iter()
                .map(|r| {
                    vec![
                        r.id.to_string(),
                        r.title.clone(),
                        r.display_state.clone(),
                        r.source.clone(),
                        "→".into(),
                        r.destination.clone(),
                        r.author.clone(),
                        reviewer_cell(&r.reviewers),
                    ]
                })
                .collect(),
        ),
    }
    Ok(())
}
