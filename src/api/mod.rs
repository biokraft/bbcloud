pub mod models;

use crate::credentials::Credentials;
use crate::error::{BbError, Result};
use crate::repo::RepoSlug;
use crate::secret::ExposeSecret;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::time::Duration;

pub const DEFAULT_BASE_URL: &str = "https://api.bitbucket.org/2.0";
const MAX_PAGES: usize = 100;
const MAX_REDIRECTS: usize = 5;

#[derive(Debug, Deserialize)]
#[serde(bound(deserialize = "T: Deserialize<'de>"))]
pub struct Page<T> {
    #[serde(default)]
    pub values: Vec<T>,
    #[serde(default)]
    pub next: Option<String>,
    #[serde(default)]
    pub size: Option<u64>,
}

pub fn repo_path(slug: &RepoSlug, suffix: &str) -> String {
    format!("/repositories/{}{}", slug.path(), suffix)
}

pub fn workspace_path(workspace: &str, suffix: &str) -> String {
    format!("/workspaces/{}{}", urlencoding::encode(workspace), suffix)
}

pub fn workspace_repos_path(workspace: &str, suffix: &str) -> String {
    format!("/repositories/{}{}", urlencoding::encode(workspace), suffix)
}

/// Bitbucket answers some endpoints — `/pullrequests/{id}/diff` among them —
/// with a 302 to another url on the same origin, so redirects have to be
/// followed or those commands fail outright. They are followed only within the
/// same origin: the Authorization header is attached to every request this
/// client makes, and it must never be replayed to another host.
fn same_origin_redirect_policy() -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(|attempt| {
        if attempt.previous().len() > MAX_REDIRECTS {
            return attempt.stop();
        }
        match attempt.previous().last() {
            Some(previous) if previous.origin() == attempt.url().origin() => attempt.follow(),
            _ => attempt.stop(),
        }
    })
}

pub struct Client {
    http: reqwest::Client,
    /// Parsed once at construction. Re-parsing a string on every request would
    /// re-validate a value already known good, and the fallback below would be
    /// a branch that can never fire.
    base_url: reqwest::Url,
    auth_header: crate::secret::SecretString,
}

impl Client {
    pub fn new(creds: Credentials, base_url: String) -> Result<Self> {
        let http = reqwest::Client::builder()
            .redirect(same_origin_redirect_policy())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .user_agent(concat!("bb-cli/", env!("CARGO_PKG_VERSION")))
            .build()?;

        let base_url = reqwest::Url::parse(base_url.trim_end_matches('/'))
            .map_err(|e| BbError::Config(format!("invalid Bitbucket API base URL: {e}")))?;

        Ok(Self {
            http,
            base_url,
            auth_header: creds.basic_header(),
        })
    }

    pub fn from_env(creds: Credentials) -> Result<Self> {
        let base = std::env::var("BB_API_BASE").unwrap_or_else(|_| DEFAULT_BASE_URL.to_string());
        Self::new(creds, base)
    }

    /// Resolves a path or absolute url against the base and refuses anything on
    /// another origin. The `Authorization` header is attached to every request
    /// this client makes, so a url from a `next` link pointing off-origin is
    /// the one shape that could hand the token to a third party.
    fn url(&self, path_or_url: &str) -> Result<String> {
        let candidate = if path_or_url.starts_with("http://") || path_or_url.starts_with("https://")
        {
            reqwest::Url::parse(path_or_url)
                .map_err(|e| BbError::Config(format!("invalid Bitbucket API URL: {e}")))?
        } else {
            // Concatenate rather than `Url::join`: join treats a leading `/` as an
            // absolute path and drops the base's `/2.0` prefix.
            let base = self.base_url.as_str().trim_end_matches('/');
            let sep = if path_or_url.starts_with('/') {
                ""
            } else {
                "/"
            };
            reqwest::Url::parse(&format!("{base}{sep}{path_or_url}"))
                .map_err(|e| BbError::Config(format!("invalid Bitbucket API URL: {e}")))?
        };

        if candidate.origin() != self.base_url.origin() {
            return Err(BbError::Config(
                "refusing to send Bitbucket credentials to another origin".into(),
            ));
        }
        Ok(candidate.to_string())
    }

    fn request(&self, method: reqwest::Method, path: &str) -> Result<reqwest::RequestBuilder> {
        Ok(self
            .http
            .request(method, self.url(path)?)
            .header(
                reqwest::header::AUTHORIZATION,
                self.auth_header.expose_secret(),
            )
            .header(reqwest::header::ACCEPT, "application/json"))
    }

    /// Turns a non-success response into a `BbError`, preferring the API's own
    /// error message over the raw body so nothing unexpected is echoed.
    async fn check(response: reqwest::Response) -> Result<reqwest::Response> {
        let status = response.status();
        if status.is_success() {
            return Ok(response);
        }
        match status.as_u16() {
            401 => return Err(BbError::Auth),
            404 => return Err(BbError::NotFound),
            429 => {
                return Err(BbError::Api {
                    status: 429,
                    message: "rate limited by bitbucket — retry shortly".into(),
                })
            }
            _ => {}
        }

        let code = status.as_u16();
        let body = response.text().await.unwrap_or_default();
        let api_message = serde_json::from_str::<serde_json::Value>(&body)
            .ok()
            .and_then(|v| {
                v.get("error")
                    .and_then(|e| e.get("message"))
                    .and_then(|m| m.as_str())
                    .map(str::to_string)
            });

        let message = if code == 403 {
            match api_message {
                Some(api_message) => format!(
                    "forbidden — {api_message} — the token may lack the required scope; see the scope table in the README"
                ),
                None => "forbidden — the token may lack the required scope; see the scope table in the README".into(),
            }
        } else {
            api_message.unwrap_or_else(|| {
                status
                    .canonical_reason()
                    .unwrap_or("request failed")
                    .to_string()
            })
        };

        Err(BbError::Api {
            status: code,
            message,
        })
    }

    pub async fn get_json<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let response = Self::check(self.request(reqwest::Method::GET, path)?.send().await?).await?;
        Ok(response.json::<T>().await?)
    }

    pub async fn get_text(&self, path: &str) -> Result<String> {
        let response = Self::check(self.request(reqwest::Method::GET, path)?.send().await?).await?;
        Ok(response.text().await?)
    }

    pub async fn post_json<T: DeserializeOwned, B: Serialize + ?Sized>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T> {
        let response = Self::check(
            self.request(reqwest::Method::POST, path)?
                .json(body)
                .send()
                .await?,
        )
        .await?;
        Ok(response.json::<T>().await?)
    }

    pub async fn post_empty(&self, path: &str) -> Result<()> {
        Self::check(self.request(reqwest::Method::POST, path)?.send().await?).await?;
        Ok(())
    }

    pub async fn put_json<T: DeserializeOwned, B: Serialize + ?Sized>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T> {
        let response = Self::check(
            self.request(reqwest::Method::PUT, path)?
                .json(body)
                .send()
                .await?,
        )
        .await?;
        Ok(response.json::<T>().await?)
    }

    pub async fn delete(&self, path: &str) -> Result<()> {
        Self::check(self.request(reqwest::Method::DELETE, path)?.send().await?).await?;
        Ok(())
    }

    pub async fn paginate<T: DeserializeOwned>(&self, path: &str) -> Result<Vec<T>> {
        self.paginate_limited(path, usize::MAX).await
    }

    /// Paginates, stopping once `limit` values have been collected.
    ///
    /// `limit` bounds how many values come back, never how many pages are
    /// walked. A caller that filters above this layer — `pr list --limit 1
    /// --build-status failed` — still needs every page, because a match may sit
    /// on page three; the cap that belongs on the wire is `MAX_PAGES`, which
    /// stops a `next` chain that never ends.
    pub async fn paginate_limited<T: DeserializeOwned>(
        &self,
        path: &str,
        limit: usize,
    ) -> Result<Vec<T>> {
        let mut collected = Vec::new();
        let mut next = Some(path.to_string());
        let mut pages = 0;
        let mut seen: HashSet<String> = HashSet::new();

        while let Some(target) = next {
            if pages >= MAX_PAGES || collected.len() >= limit {
                break;
            }
            // A `next` link that repeats an already-fetched url would otherwise
            // refetch the same page up to MAX_PAGES times and silently return
            // duplicated values. Compare resolved urls so a relative path and
            // the absolute url it resolves to are recognized as the same page.
            if !seen.insert(self.url(&target)?) {
                break;
            }

            let page: Page<T> = self.get_json(&target).await?;
            collected.extend(page.values);
            next = page.next;
            pages += 1;
        }

        Ok(collected)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn workspace_path_encodes_the_slug_exactly_once() {
        assert_eq!(
            workspace_path("acme", "/projects"),
            "/workspaces/acme/projects"
        );
        assert_eq!(
            workspace_path("a c/me", "/projects"),
            "/workspaces/a%20c%2Fme/projects"
        );
    }

    fn client_for(base: &str) -> Client {
        let creds = Credentials {
            email: "dev@example.com".into(),
            token: crate::secret::SecretString::from("s3cr3t-token"),
        };
        Client::new(creds, base.to_string()).unwrap()
    }

    #[test]
    fn url_keeps_the_version_prefix_of_the_base() {
        let client = client_for(DEFAULT_BASE_URL);
        assert_eq!(
            client.url("/user").unwrap(),
            "https://api.bitbucket.org/2.0/user"
        );
        assert_eq!(
            client
                .url("/repositories/acme/api/pullrequests?pagelen=50")
                .unwrap(),
            "https://api.bitbucket.org/2.0/repositories/acme/api/pullrequests?pagelen=50"
        );
    }

    #[test]
    fn url_tolerates_a_trailing_slash_on_the_base() {
        let client = client_for("https://api.bitbucket.org/2.0/");
        assert_eq!(
            client.url("/user").unwrap(),
            "https://api.bitbucket.org/2.0/user"
        );
    }

    #[test]
    fn url_passes_an_absolute_same_origin_next_link_through() {
        let client = client_for(DEFAULT_BASE_URL);
        let next = "https://api.bitbucket.org/2.0/repositories/acme?page=2";
        assert_eq!(client.url(next).unwrap(), next);
    }

    #[test]
    fn url_refuses_another_origin() {
        let client = client_for(DEFAULT_BASE_URL);
        assert!(client.url("https://evil.example/2.0/user").is_err());
    }

    #[test]
    fn workspace_repos_path_encodes_the_slug_exactly_once() {
        assert_eq!(workspace_repos_path("acme", ""), "/repositories/acme");
        assert_eq!(
            workspace_repos_path("a c/me", "?pagelen=100"),
            "/repositories/a%20c%2Fme?pagelen=100"
        );
    }
}
