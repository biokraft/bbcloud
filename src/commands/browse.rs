use crate::error::Result;
use crate::output::{self, Format};
use crate::repo::{self, RepoSlug};

#[derive(Debug, Clone, Copy)]
pub enum BrowseTarget {
    Pr(u64),
    Branches,
}

pub fn url_for(slug: &RepoSlug, target: Option<&BrowseTarget>) -> String {
    let base = slug.browse_url();
    match target {
        Some(BrowseTarget::Pr(id)) => format!("{base}/pull-requests/{id}"),
        Some(BrowseTarget::Branches) => format!("{base}/branches"),
        None => base,
    }
}

pub fn browse(
    repo_arg: Option<&str>,
    target: Option<BrowseTarget>,
    print_only: bool,
    format: Format,
) -> Result<()> {
    // Parsing happens first: an invalid or hostile value never reaches a spawn.
    let slug = repo::resolve(repo_arg)?;
    let url = url_for(&slug, target.as_ref());

    if format.is_json() {
        return output::print_json(&serde_json::json!({ "url": url }));
    }

    println!("{url}");

    if !print_only {
        // The url is passed as a single argument, not through a shell.
        if let Err(err) = open::that_detached(&url) {
            output::warn(&format!("cannot open a browser: {err}"));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slug() -> RepoSlug {
        RepoSlug {
            workspace: "acme".into(),
            repo: "widgets".into(),
        }
    }

    #[test]
    fn repo_url_has_no_suffix() {
        assert_eq!(url_for(&slug(), None), "https://bitbucket.org/acme/widgets");
    }

    #[test]
    fn pr_url_includes_the_id() {
        assert_eq!(
            url_for(&slug(), Some(&BrowseTarget::Pr(7))),
            "https://bitbucket.org/acme/widgets/pull-requests/7"
        );
    }

    #[test]
    fn branches_url() {
        assert!(url_for(&slug(), Some(&BrowseTarget::Branches)).ends_with("/branches"));
    }

    /// Pinning the leading-dash analysis: a workspace beginning with `-` is
    /// accepted by `valid_segment`, but `browse_url` always prefixes it with
    /// `https://bitbucket.org/`, so the dash is never the first character of
    /// the string handed to `open::that_detached`, and it is never its own
    /// argv element (the whole url is one argument). It cannot be mistaken
    /// for a flag by `open`/`xdg-open`.
    #[test]
    fn dash_prefixed_workspace_produces_a_well_formed_url() {
        let slug = RepoSlug {
            workspace: "-rf".into(),
            repo: "widgets".into(),
        };
        let url = url_for(&slug, None);
        assert_eq!(url, "https://bitbucket.org/-rf/widgets");
        assert!(url.starts_with("https://"));
    }
}
