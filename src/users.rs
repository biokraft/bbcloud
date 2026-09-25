use crate::api::models::{EffectiveReviewer, User};
use crate::api::{repo_path, Client};
use crate::error::{BbError, Result};
use crate::output;
use crate::repo::RepoSlug;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Deserialize)]
struct Membership {
    user: Option<User>,
}

#[derive(Debug, Deserialize)]
struct RepoPermission {
    user: Option<User>,
}

/// What a pool entry proves about repository access. Direct repository
/// permissions are explicit; workspace membership and default-reviewer status
/// say nothing about access to the selected repository.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Eligibility {
    /// Directly listed in the repository's permission configuration.
    Explicit,
    /// A plausible reviewer, with no proof of access to this repository.
    Unknown,
}

#[derive(Debug, Clone)]
pub(crate) struct PoolEntry {
    pub user: User,
    pub sources: Vec<String>,
    pub eligibility: Eligibility,
}

#[derive(Debug, Clone)]
pub(crate) struct UserPool {
    pub entries: Vec<PoolEntry>,
    pub incomplete: Vec<String>,
}

pub(crate) fn uuid_user(query: &str) -> Option<User> {
    let query = query.trim();
    if query.starts_with('{') && query.ends_with('}') {
        Some(User {
            uuid: Some(query.to_string()),
            account_id: None,
            display_name: None,
            nickname: None,
        })
    } else {
        None
    }
}

impl UserPool {
    pub(crate) async fn load(client: &Client, slug: &RepoSlug) -> Result<Self> {
        let mut pool = Self {
            entries: Vec::new(),
            incomplete: Vec::new(),
        };

        match client
            .paginate::<Membership>(&format!(
                "/workspaces/{}/members?pagelen=100",
                slug.workspace
            ))
            .await
        {
            Ok(memberships) => {
                for membership in memberships {
                    if let Some(user) = membership.user {
                        pool.add(user, "workspace", Eligibility::Unknown);
                    }
                }
            }
            Err(BbError::Auth) => return Err(BbError::Auth),
            Err(BbError::Api { status: 403, .. }) => {
                pool.incomplete.push("workspace".into());
            }
            Err(other) => return Err(other),
        }

        match client
            .paginate::<RepoPermission>(&repo_path(slug, "/permissions-config/users?pagelen=100"))
            .await
        {
            Ok(permissions) => {
                for permission in permissions {
                    if let Some(user) = permission.user {
                        pool.add(user, "repository", Eligibility::Explicit);
                    }
                }
            }
            Err(BbError::Auth) => return Err(BbError::Auth),
            Err(BbError::Api { status: 403, .. }) => {
                pool.incomplete.push("repository".into());
            }
            Err(other) => return Err(other),
        }

        match client
            .paginate::<EffectiveReviewer>(&repo_path(
                slug,
                "/effective-default-reviewers?pagelen=100",
            ))
            .await
        {
            Ok(reviewers) => {
                for reviewer in reviewers {
                    if let Some(user) = reviewer.user {
                        pool.add(user, "default_reviewer", Eligibility::Unknown);
                    }
                }
            }
            Err(BbError::Auth) => return Err(BbError::Auth),
            Err(BbError::Api { status: 403, .. }) => {
                pool.incomplete.push("default_reviewer".into());
            }
            Err(other) => return Err(other),
        }

        Ok(pool)
    }

    fn add(&mut self, user: User, source: &str, eligibility: Eligibility) {
        if let Some(entry) = self
            .entries
            .iter_mut()
            .find(|entry| same_identity(&entry.user, &user))
        {
            merge_fields(&mut entry.user, user);
            if !entry.sources.iter().any(|value| value == source) {
                entry.sources.push(source.into());
            }
            if eligibility == Eligibility::Explicit {
                entry.eligibility = Eligibility::Explicit;
            }
            return;
        }
        self.entries.push(PoolEntry {
            user,
            sources: vec![source.into()],
            eligibility,
        });
    }

    pub(crate) fn resolve(&self, query: &str, extra: &[User]) -> Result<User> {
        self.resolve_inner(query, extra, false)
    }

    /// Name resolution for commands that will write a reviewer. An incomplete
    /// pool can hide the person the caller meant, so a name is never good enough
    /// there; a `{uuid}` bypasses the pool entirely.
    pub(crate) fn resolve_for_write(&self, query: &str, extra: &[User]) -> Result<User> {
        self.resolve_inner(query, extra, true)
    }

    fn resolve_inner(&self, query: &str, extra: &[User], mutating: bool) -> Result<User> {
        let query = query.trim();
        if query.is_empty() {
            return Err(BbError::Config("empty user name".into()));
        }
        if let Some(user) = uuid_user(query) {
            return Ok(user);
        }
        if mutating && !self.incomplete.is_empty() {
            return Err(BbError::Config(format!(
                "cannot resolve `{query}` by name: {} could not be read ({}) — pass a `{{uuid}}` to be exact",
                self.incomplete.join(", "),
                "an unreadable list may hide the person you mean"
            )));
        }

        let needle = query.to_lowercase();
        let mut users: Vec<User> = self
            .entries
            .iter()
            .map(|entry| entry.user.clone())
            .collect();
        users.extend(extra.iter().cloned());
        let mut seen = HashSet::new();
        users.retain(|user| {
            let Some(identity) = identity(user) else {
                return false;
            };
            seen.insert(identity)
        });
        let mut found: Vec<User> = users
            .into_iter()
            .filter(|user| matches(user, &needle))
            .collect();
        if found.iter().any(|user| is_exact(user, &needle)) {
            found.retain(|user| is_exact(user, &needle));
        }

        match found.len() {
            1 => Ok(found.remove(0)),
            0 => {
                if !self.incomplete.is_empty() {
                    output::warn(
                        "some user lists could not be read, so the name search may be \
                         incomplete — pass a `{uuid}` to be exact",
                    );
                }
                Err(BbError::Config(format!(
                    "no user matching `{query}` — pass a `{{uuid}}` to be exact"
                )))
            }
            _ => {
                let names: Vec<&str> = found.iter().map(|user| user.name()).collect();
                Err(BbError::Config(format!(
                    "`{query}` matches {} people: {} — pass a `{{uuid}}` to be exact",
                    names.len(),
                    names.join(", ")
                )))
            }
        }
    }
}

fn identity(user: &User) -> Option<String> {
    user.uuid
        .as_deref()
        .map(|uuid| format!("uuid:{uuid}"))
        .or_else(|| {
            user.account_id
                .as_deref()
                .map(|account_id| format!("account:{account_id}"))
        })
}

fn same_identity(left: &User, right: &User) -> bool {
    match (identity(left), identity(right)) {
        (Some(left), Some(right)) => left == right,
        _ => false,
    }
}

/// The api returns every field as optional, and different pools populate
/// different subsets. A later, richer representation must not erase what an
/// earlier one knew.
fn merge_fields(existing: &mut User, incoming: User) {
    if existing.uuid.is_none() {
        existing.uuid = incoming.uuid;
    }
    if existing.account_id.is_none() {
        existing.account_id = incoming.account_id;
    }
    if existing.display_name.is_none() {
        existing.display_name = incoming.display_name;
    }
    if existing.nickname.is_none() {
        existing.nickname = incoming.nickname;
    }
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

pub async fn current_user(client: &Client) -> Result<User> {
    client.get_json("/user").await
}

pub(crate) async fn load_user_pool(client: &Client, slug: &RepoSlug) -> Result<UserPool> {
    UserPool::load(client, slug).await
}

pub async fn resolve_user(
    client: &Client,
    slug: &RepoSlug,
    query: &str,
    extra: &[User],
) -> Result<User> {
    if let Some(user) = uuid_user(query) {
        return Ok(user);
    }
    UserPool::load(client, slug).await?.resolve(query, extra)
}
