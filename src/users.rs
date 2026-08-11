use crate::api::models::User;
use crate::api::{repo_path, Client};
use crate::error::{BbError, Result};
use crate::repo::RepoSlug;
use serde::Deserialize;

/// `/workspaces/{ws}/members` wraps each user in a membership object, unlike
/// `/default-reviewers`, which returns users directly.
#[derive(Debug, Deserialize)]
struct Membership {
    user: Option<User>,
}

pub async fn current_user(client: &Client) -> Result<User> {
    client.get_json("/user").await
}

/// Everyone `query` could plausibly mean, deduplicated by uuid.
async fn candidates(client: &Client, slug: &RepoSlug, extra: &[User]) -> Result<Vec<User>> {
    let mut pool: Vec<User> = Vec::new();

    // The token may not carry workspace scope. That is not fatal: the smaller
    // pools below still cover the common cases.
    match client
        .paginate::<Membership>(&format!(
            "/workspaces/{}/members?pagelen=100",
            slug.workspace
        ))
        .await
    {
        Ok(memberships) => pool.extend(memberships.into_iter().filter_map(|m| m.user)),
        Err(BbError::Api { status: 403, .. }) | Err(BbError::Auth) => {}
        Err(other) => return Err(other),
    }

    let defaults: Vec<User> = client
        .paginate(&repo_path(slug, "/default-reviewers?pagelen=100"))
        .await?;
    pool.extend(defaults);

    for user in extra {
        pool.push(User {
            uuid: user.uuid.clone(),
            account_id: user.account_id.clone(),
            display_name: user.display_name.clone(),
            nickname: user.nickname.clone(),
        });
    }

    let mut seen: Vec<String> = Vec::new();
    pool.retain(|user| match user.uuid.as_deref() {
        Some(uuid) => {
            let fresh = !seen.iter().any(|s| s == uuid);
            if fresh {
                seen.push(uuid.to_string());
            }
            fresh
        }
        None => true,
    });

    Ok(pool)
}

fn matches(user: &User, needle: &str) -> bool {
    [user.display_name.as_deref(), user.nickname.as_deref()]
        .into_iter()
        .flatten()
        .any(|field| field.to_lowercase().contains(needle))
}

fn is_exact(user: &User, needle: &str) -> bool {
    [user.display_name.as_deref(), user.nickname.as_deref()]
        .into_iter()
        .flatten()
        .any(|field| field.to_lowercase() == needle)
}

/// One human-typed name to one user.
///
/// A `{uuid}` is already exact and is taken verbatim, with no api call. Anything
/// else is matched case-insensitively as a substring of the display name or
/// nickname. Emails are not accepted: a reviewer is written to the api as a uuid,
/// and bitbucket's member listings do not expose email addresses, so an email
/// could never be resolved — failing here beats failing inside a write.
pub async fn resolve_user(
    client: &Client,
    slug: &RepoSlug,
    query: &str,
    extra: &[User],
) -> Result<User> {
    let query = query.trim();
    if query.is_empty() {
        return Err(BbError::Config("empty user name".into()));
    }
    if query.starts_with('{') && query.ends_with('}') {
        return Ok(User {
            uuid: Some(query.to_string()),
            account_id: None,
            display_name: None,
            nickname: None,
        });
    }

    let needle = query.to_lowercase();
    let pool = candidates(client, slug, extra).await?;

    // An exact name wins outright, or a workspace holding both "ana" and
    // "anastasia" makes "ana" unaddressable forever.
    let mut found: Vec<User> = pool.into_iter().filter(|u| matches(u, &needle)).collect();
    if found.iter().any(|u| is_exact(u, &needle)) {
        found.retain(|u| is_exact(u, &needle));
    }

    match found.len() {
        1 => Ok(found.remove(0)),
        0 => Err(BbError::Config(format!(
            "no user matching `{query}` — pass a `{{uuid}}` to be exact"
        ))),
        _ => {
            let names: Vec<&str> = found.iter().map(|u| u.name()).collect();
            Err(BbError::Config(format!(
                "`{query}` matches {} people: {} — pass a `{{uuid}}` to be exact",
                names.len(),
                names.join(", ")
            )))
        }
    }
}
