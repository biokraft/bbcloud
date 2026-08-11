pub mod models;

use crate::credentials::Credentials;
use crate::error::{BbError, Result};
use crate::repo::RepoSlug;
use crate::secret::ExposeSecret;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
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
    base_url: String,
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

        Ok(Self {
            http,
            base_url: base_url.trim_end_matches('/').to_string(),
            auth_header: creds.basic_header(),
        })
    }

    pub fn from_env(creds: Credentials) -> Result<Self> {
        let base = std::env::var("BB_API_BASE").unwrap_or_else(|_| DEFAULT_BASE_URL.to_string());
        Self::new(creds, base)
    }

    fn url(&self, path_or_url: &str) -> String {
        if path_or_url.starts_with("http://") || path_or_url.starts_with("https://") {
            path_or_url.to_string()
        } else {
            format!("{}{}", self.base_url, path_or_url)
        }
    }

    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        self.http
            .request(method, self.url(path))
            .header(
                reqwest::header::AUTHORIZATION,
                self.auth_header.expose_secret(),
            )
            .header(reqwest::header::ACCEPT, "application/json")
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
            403 => {
                return Err(BbError::Api {
                    status: 403,
                    message: "forbidden — the token may lack the required scope".into(),
                })
            }
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
        let message = serde_json::from_str::<serde_json::Value>(&body)
            .ok()
            .and_then(|v| {
                v.get("error")
                    .and_then(|e| e.get("message"))
                    .and_then(|m| m.as_str())
                    .map(str::to_string)
            })
            .unwrap_or_else(|| {
                status
                    .canonical_reason()
                    .unwrap_or("request failed")
                    .to_string()
            });

        Err(BbError::Api {
            status: code,
            message,
        })
    }

    pub async fn get_json<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let response = Self::check(self.request(reqwest::Method::GET, path).send().await?).await?;
        Ok(response.json::<T>().await?)
    }

    pub async fn get_text(&self, path: &str) -> Result<String> {
        let response = Self::check(self.request(reqwest::Method::GET, path).send().await?).await?;
        Ok(response.text().await?)
    }

    pub async fn post_json<T: DeserializeOwned, B: Serialize + ?Sized>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T> {
        let response = Self::check(
            self.request(reqwest::Method::POST, path)
                .json(body)
                .send()
                .await?,
        )
        .await?;
        Ok(response.json::<T>().await?)
    }

    pub async fn post_empty(&self, path: &str) -> Result<()> {
        Self::check(self.request(reqwest::Method::POST, path).send().await?).await?;
        Ok(())
    }

    pub async fn put_json<T: DeserializeOwned, B: Serialize + ?Sized>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T> {
        let response = Self::check(
            self.request(reqwest::Method::PUT, path)
                .json(body)
                .send()
                .await?,
        )
        .await?;
        Ok(response.json::<T>().await?)
    }

    pub async fn delete(&self, path: &str) -> Result<()> {
        Self::check(self.request(reqwest::Method::DELETE, path).send().await?).await?;
        Ok(())
    }

    pub async fn paginate<T: DeserializeOwned>(&self, path: &str) -> Result<Vec<T>> {
        let mut collected = Vec::new();
        let mut next = Some(path.to_string());
        let mut pages = 0;
        let mut seen: Vec<String> = Vec::new();

        while let Some(target) = next {
            if pages >= MAX_PAGES {
                break;
            }
            // A `next` link that repeats an already-fetched url would otherwise
            // refetch the same page up to MAX_PAGES times and silently return
            // duplicated values. Compare resolved urls so a relative path and
            // the absolute url it resolves to are recognized as the same page.
            let resolved = self.url(&target);
            if seen.contains(&resolved) {
                break;
            }
            seen.push(resolved);

            let page: Page<T> = self.get_json(&target).await?;
            collected.extend(page.values);
            next = page.next;
            pages += 1;
        }

        Ok(collected)
    }
}
