use crate::api::models::{EffectiveReviewer, User};
use crate::api::{repo_path, Client};
use crate::error::{BbError, Result};
use crate::output;
use crate::repo::RepoSlug;
use serde::Deserialize;
use std::collections::HashSet;

#[derive(Debug, Deserialize)]
struct Membership {
    user: Option<User>,
}

#[derive(Debug, Deserialize)]
struct RepoPermission {
    user: Option<User>,
}

#[derive(Debug, Clone)]
pub(crate) struct PoolEntry {
    pub user: User,
    pub sources: Vec<String>,
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
                        pool.add(user, "workspace");
                    }
                }
            }
            Err(BbError::Auth) => return Err(BbError::Auth),
            Err(BbError::Api { status: 403, .. }) | Err(BbError::NotFound) => {
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
                        pool.add(user, "repository");
                    }
                }
            }
            Err(BbError::Auth) => return Err(BbError::Auth),
            Err(BbError::Api { status: 403, .. }) | Err(BbError::NotFound) => {
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
                        pool.add(user, "default_reviewer");
                    }
                }
            }
            Err(BbError::Auth) => return Err(BbError::Auth),
            Err(BbError::Api { status: 403, .. }) | Err(BbError::NotFound) => {
                pool.incomplete.push("default_reviewer".into());
            }
            Err(other) => return Err(other),
        }

        Ok(pool)
    }

    fn add(&mut self, user: User, source: &str) {
        if let Some(uuid) = user.uuid.as_deref() {
            if let Some(entry) = self
                .entries
                .iter_mut()
                .find(|entry| entry.user.uuid.as_deref() == Some(uuid))
            {
                if !entry.sources.iter().any(|value| value == source) {
                    entry.sources.push(source.into());
                }
                return;
            }
        }
        self.entries.push(PoolEntry {
            user,
            sources: vec![source.into()],
        });
    }

    pub(crate) fn resolve(&self, query: &str, extra: &[User]) -> Result<User> {
        let query = query.trim();
        if query.is_empty() {
            return Err(BbError::Config("empty user name".into()));
        }
        if let Some(user) = uuid_user(query) {
            return Ok(user);
        }

        let needle = query.to_lowercase();
        let mut users: Vec<User> = self
            .entries
            .iter()
            .map(|entry| entry.user.clone())
            .collect();
        users.extend(extra.iter().cloned());
        let mut seen = HashSet::new();
        users.retain(|user| match user.uuid.as_deref() {
            Some(uuid) => seen.insert(uuid.to_string()),
            None => true,
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
