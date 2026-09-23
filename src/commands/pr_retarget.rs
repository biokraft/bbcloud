//! `bb pr retarget`: point an open pull request at a different destination branch.
//!
//! The api only lets the destination move — a pull request's source branch is
//! fixed for its lifetime — so there is no flag for the other side. The update
//! rides the same `PUT` that edits a pull request's title, which is why the
//! existing title is read back and resent: a `PUT` without it is rejected.

use crate::api::models::PullRequest;
use crate::commands::pr::Ctx;
use crate::error::{BbError, Result};
use crate::output::{self, Format};
use serde::Serialize;

#[derive(Serialize)]
struct RetargetRow {
    id: u64,
    title: String,
    source: String,
    destination: String,
    url: String,
}

impl RetargetRow {
    fn from(pr: &PullRequest) -> Self {
        Self {
            id: pr.id,
            title: pr.title.clone().unwrap_or_default(),
            source: pr.source_branch().to_string(),
            destination: pr.destination_branch().to_string(),
            url: pr.html_url().to_string(),
        }
    }
}

pub async fn run(ctx: &Ctx, id: u64, to: &str) -> Result<()> {
    let path = ctx.path(&format!("/pullrequests/{id}"));
    let pr: PullRequest = ctx.client.get_json(&path).await?;

    // A closed pull request answers the `PUT` with an unhelpful 400, so the
    // state is checked here where the message can name what is wrong.
    let state = pr.state.as_deref().unwrap_or("UNKNOWN");
    if !state.eq_ignore_ascii_case("OPEN") {
        return Err(BbError::Config(format!(
            "pull request #{id} is {state}, and only an open one can be retargeted"
        )));
    }

    let from = pr.destination_branch().to_string();
    if from == to {
        let row = RetargetRow::from(&pr);
        match ctx.format {
            Format::Json => output::print_json(&row)?,
            Format::Human => output::info(&format!(
                "pull request #{id} already targets {to} — nothing to do"
            )),
        }
        return Ok(());
    }

    let title = pr.title.clone().unwrap_or_default();
    let body = serde_json::json!({
        "title": title,
        "destination": { "branch": { "name": to } },
    });
    let updated: PullRequest = ctx.client.put_json(&path, &body).await?;

    let row = RetargetRow::from(&updated);
    match ctx.format {
        Format::Json => output::print_json(&row)?,
        Format::Human => {
            output::success(&format!("pull request #{id} retargeted {from} → {to}"));
            output::warn(
                "the diff was recomputed; inline comments on the old base may now read as outdated",
            );
        }
    }
    Ok(())
}
