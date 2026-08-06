use crate::api::models::BranchRef;
use crate::commands::pr::Ctx;
use crate::error::Result;
use crate::output::{self, Format};
use serde::Serialize;

#[derive(Debug, Serialize)]
struct BranchRow {
    branch: String,
    user: String,
    updated: String,
}

pub async fn list(
    ctx: &Ctx,
    user: Option<String>,
    name: Option<String>,
    limit: usize,
) -> Result<()> {
    let spinner = output::spinner("fetching branches");
    let branches: Vec<BranchRef> = ctx
        .client
        .paginate(&ctx.path("/refs/branches?pagelen=100&sort=-target.date"))
        .await?;
    spinner.finish_and_clear();

    let user = user.map(|u| u.to_lowercase());
    let name = name.map(|n| n.to_lowercase());

    let rows: Vec<BranchRow> = branches
        .iter()
        .filter(|b| match &name {
            Some(needle) => b.name.to_lowercase().contains(needle),
            None => true,
        })
        .filter(|b| match &user {
            Some(needle) => b.owner().to_lowercase().contains(needle),
            None => true,
        })
        .take(limit)
        .map(|b| BranchRow {
            branch: b.name.clone(),
            user: b.owner(),
            updated: b
                .target
                .as_ref()
                .and_then(|t| t.date.as_deref())
                .map(output::relative_time)
                .unwrap_or_else(|| "-".into()),
        })
        .collect();

    match ctx.format {
        Format::Json => output::print_json(&rows)?,
        Format::Human => output::print_table(
            &["BRANCH", "LAST COMMIT BY", "UPDATED"],
            rows.iter()
                .map(|r| vec![r.branch.clone(), r.user.clone(), r.updated.clone()])
                .collect(),
        ),
    }

    Ok(())
}
