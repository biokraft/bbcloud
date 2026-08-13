use crate::api::models::{BuildState, BuildStatus};
use crate::api::{self, Client};
use crate::commands::pr::Ctx;
use crate::error::Result;
use crate::output::{self, Format};
use crate::repo::RepoSlug;
use futures::stream::{self, StreamExt, TryStreamExt};
use serde::Serialize;
use std::collections::HashMap;

/// Bitbucket exposes build status only per pull request, so a column over N rows
/// costs N requests. Cap how many are in flight: enough to be fast on a busy
/// repository, few enough to stay clear of the rate limit.
const MAX_IN_FLIGHT: usize = 8;

pub async fn statuses(client: &Client, slug: &RepoSlug, id: u64) -> Result<Vec<BuildStatus>> {
    client
        .paginate(&api::repo_path(
            slug,
            &format!("/pullrequests/{id}/statuses"),
        ))
        .await
}

pub async fn statuses_for(
    client: &Client,
    slug: &RepoSlug,
    ids: &[u64],
) -> Result<HashMap<u64, Vec<BuildStatus>>> {
    stream::iter(ids.iter().copied())
        .map(|id| async move { statuses(client, slug, id).await.map(|s| (id, s)) })
        .buffer_unordered(MAX_IN_FLIGHT)
        .try_collect()
        .await
}

#[derive(Debug, Serialize)]
struct BuildReport {
    build_state: BuildState,
    statuses: Vec<BuildStatus>,
}

pub async fn run(ctx: &Ctx, id: u64) -> Result<()> {
    let spinner = output::spinner("fetching build statuses");
    let found = statuses(&ctx.client, &ctx.slug, id).await?;
    spinner.finish_and_clear();

    let report = BuildReport {
        build_state: BuildState::rollup(&found),
        statuses: found,
    };

    match ctx.format {
        Format::Json => output::print_json(&report)?,
        Format::Human => {
            output::heading(&format!(
                "build: {}",
                output::colored_cell(
                    report.build_state.label(),
                    output::tone_for(report.build_state)
                )
            ));
            if report.statuses.is_empty() {
                output::info("no build statuses");
            } else {
                output::print_table(
                    &["KEY", "NAME", "STATE", "URL"],
                    report
                        .statuses
                        .iter()
                        .map(|s| {
                            let state = BuildState::from_api(s.state.as_deref());
                            vec![
                                s.key.clone().unwrap_or_else(|| "-".into()),
                                s.name.clone().unwrap_or_else(|| "-".into()),
                                output::colored_cell(state.label(), output::tone_for(state)),
                                s.url.clone().unwrap_or_else(|| "-".into()),
                            ]
                        })
                        .collect(),
                );
            }
        }
    }
    Ok(())
}
