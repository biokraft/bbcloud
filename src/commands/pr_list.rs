use crate::api::models::{PullRequest, ReviewerState};
use crate::commands::pr::Ctx;
use crate::error::Result;
use crate::output::{self, Format};
use serde::Serialize;

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

/// `all` is a bb-level convenience, not a bitbucket state; everything else is
/// passed through upper-cased as before.
fn state_query(state: &str) -> String {
    if state.eq_ignore_ascii_case("all") {
        ALL_STATES.to_string()
    } else {
        state.to_uppercase()
    }
}

pub async fn list(ctx: &Ctx, destination: Option<String>, state: String) -> Result<()> {
    let spinner = output::spinner("fetching pull requests");
    let prs: Vec<PullRequest> = ctx
        .client
        .paginate(&ctx.path(&format!(
            "/pullrequests?state={}&pagelen=50&fields={REVIEWER_FIELDS}",
            urlencoding::encode(&state_query(&state))
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
