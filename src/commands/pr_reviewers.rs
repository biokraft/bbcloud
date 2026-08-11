use crate::api::models::{PullRequest, ReviewerRef, ReviewerState, User};
use crate::commands::pr::Ctx;
use crate::error::{BbError, Result};
use crate::output::{self, Format};
use crate::users::resolve_user;

async fn fetch(ctx: &Ctx, id: u64) -> Result<PullRequest> {
    ctx.client
        .get_json(&ctx.path(&format!("/pullrequests/{id}")))
        .await
}

fn render(ctx: &Ctx, states: &[ReviewerState]) -> Result<()> {
    match ctx.format {
        Format::Json => output::print_json(&states)?,
        Format::Human => output::print_table(
            &["NAME", "STATE"],
            states
                .iter()
                .map(|s| {
                    vec![
                        s.name.clone(),
                        // The serialized name is the same vocabulary the --json
                        // output uses, so humans and scripts read one set of words.
                        serde_json::to_value(s.state)
                            .ok()
                            .and_then(|v| v.as_str().map(str::to_string))
                            .unwrap_or_else(|| "pending".into()),
                    ]
                })
                .collect(),
        ),
    }
    Ok(())
}

pub async fn list(ctx: &Ctx, id: u64) -> Result<()> {
    let pr = fetch(ctx, id).await?;
    render(ctx, &pr.reviewer_states())
}

fn split_names(names: &str) -> Vec<&str> {
    names
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect()
}

/// Resolves every name before any write, so a typo in the second name cannot
/// leave a half-applied change.
async fn resolve_all(ctx: &Ctx, names: &str, pool: &[User]) -> Result<Vec<User>> {
    let requested = split_names(names);
    if requested.is_empty() {
        return Err(BbError::Config("no reviewer name given".into()));
    }
    let mut resolved = Vec::new();
    for name in requested {
        resolved.push(resolve_user(&ctx.client, &ctx.slug, name, pool).await?);
    }
    Ok(resolved)
}

/// There is no add-reviewer or remove-reviewer endpoint, so the whole set is
/// written back. `title` is included because the api rejects a PUT without it;
/// every other field is omitted and left untouched.
async fn write_reviewers(ctx: &Ctx, id: u64, pr: &PullRequest, uuids: Vec<String>) -> Result<()> {
    let body = serde_json::json!({
        "title": pr.title.clone().unwrap_or_default(),
        "reviewers": uuids
            .into_iter()
            .map(|uuid| ReviewerRef { uuid })
            .collect::<Vec<_>>(),
    });
    let updated: PullRequest = ctx
        .client
        .put_json(&ctx.path(&format!("/pullrequests/{id}")), &body)
        .await?;
    render(ctx, &updated.reviewer_states())
}

fn current_uuids(pr: &PullRequest) -> Vec<String> {
    pr.reviewers.iter().filter_map(|r| r.uuid.clone()).collect()
}

pub async fn add(ctx: &Ctx, id: u64, names: &str) -> Result<()> {
    let pr = fetch(ctx, id).await?;
    let resolved = resolve_all(ctx, names, &pr.reviewers).await?;

    let mut uuids = current_uuids(&pr);
    let mut added: Vec<String> = Vec::new();
    let mut already: Vec<String> = Vec::new();
    for user in &resolved {
        let uuid = user
            .uuid
            .clone()
            .ok_or_else(|| BbError::Config(format!("`{}` has no uuid to tag", user.name())))?;
        if uuids.contains(&uuid) {
            already.push(user.name().to_string());
        } else {
            uuids.push(uuid);
            added.push(user.name().to_string());
        }
    }

    if added.is_empty() {
        if !ctx.format.is_json() {
            output::info(&format!("already a reviewer: {}", already.join(", ")));
        }
        return render(ctx, &pr.reviewer_states());
    }

    if !ctx.format.is_json() {
        output::success(&format!("added {}", added.join(", ")));
        if !already.is_empty() {
            output::info(&format!("already a reviewer: {}", already.join(", ")));
        }
    }
    write_reviewers(ctx, id, &pr, uuids).await
}

pub async fn remove(ctx: &Ctx, id: u64, names: &str) -> Result<()> {
    let pr = fetch(ctx, id).await?;
    let resolved = resolve_all(ctx, names, &pr.reviewers).await?;

    let mut uuids = current_uuids(&pr);
    let mut removed: Vec<String> = Vec::new();
    for user in &resolved {
        // A silent no-op would let "remove Raigon" look like it worked when it
        // matched nobody on this pull request.
        let uuid = user
            .uuid
            .as_deref()
            .filter(|uuid| uuids.iter().any(|u| u == uuid))
            .ok_or_else(|| {
                BbError::Config(format!("`{}` is not a reviewer on #{id}", user.name()))
            })?
            .to_string();
        uuids.retain(|u| *u != uuid);
        removed.push(user.name().to_string());
    }

    if !ctx.format.is_json() {
        output::success(&format!("removed {}", removed.join(", ")));
    }
    write_reviewers(ctx, id, &pr, uuids).await
}
