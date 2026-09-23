//! `bb pr edit`: change an open pull request's title or description in place.
//!
//! This rides the same `PUT` as `pr retarget` and `pr reviewers`. The title is
//! always sent, because the api rejects a `PUT` without one. The description is
//! sent only when it changed, so fixing a typo in the title never rewrites text
//! nobody touched.

use crate::api::models::PullRequest;
use crate::commands::pr::Ctx;
use crate::error::{BbError, Result};
use crate::output::{self, Format};
use serde::Serialize;

#[derive(Debug, Default)]
pub struct EditArgs {
    pub id: u64,
    pub title: Option<String>,
    pub description: Option<String>,
    pub description_stdin: bool,
}

/// The new text a caller asked for. `None` leaves that field alone.
#[derive(Debug, Default, PartialEq, Eq)]
struct Requested {
    title: Option<String>,
    description: Option<String>,
}

/// Only the fields that differ from what the pull request says now.
#[derive(Debug, Default, PartialEq, Eq)]
struct Plan {
    title: Option<String>,
    description: Option<String>,
}

impl Plan {
    fn changed(&self) -> Vec<&'static str> {
        let mut changed = Vec::new();
        if self.title.is_some() {
            changed.push("title");
        }
        if self.description.is_some() {
            changed.push("description");
        }
        changed
    }
}

/// Drops every requested value that matches the current one. Trailing
/// whitespace is ignored for the description: an editor or `echo` adds a
/// final newline, and that alone is not an edit anyone meant.
fn plan(current_title: &str, current_description: &str, requested: Requested) -> Plan {
    Plan {
        title: requested.title.filter(|t| t != current_title),
        description: requested
            .description
            .filter(|d| d.trim_end() != current_description.trim_end()),
    }
}

fn clean_title(raw: &str) -> Result<String> {
    let title = raw.trim();
    if title.is_empty() {
        return Err(BbError::Config("the title cannot be empty".into()));
    }
    Ok(title.to_string())
}

/// What the flags ask for, or `None` when no content flag was given and the
/// caller should be prompted instead.
fn from_flags(args: &EditArgs) -> Result<Option<Requested>> {
    if args.title.is_none() && args.description.is_none() && !args.description_stdin {
        return Ok(None);
    }
    let title = args.title.as_deref().map(clean_title).transpose()?;
    let description = if args.description_stdin {
        let mut buf = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut buf)?;
        Some(buf.trim_end_matches('\n').to_string())
    } else {
        args.description
            .as_deref()
            .map(|d| d.trim_end_matches('\n').to_string())
    };
    Ok(Some(Requested { title, description }))
}

/// Left uncovered on purpose: it needs a terminal, and the decision it feeds
/// is made by `plan`, which is tested.
fn prompt(current_title: &str, current_description: &str) -> Result<Requested> {
    let title = inquire::Text::new("title:")
        .with_initial_value(current_title)
        .prompt()
        .map_err(|e| BbError::Config(format!("cancelled: {e}")))?;
    let description = inquire::Editor::new("description:")
        .with_predefined_text(current_description)
        .with_file_extension(".md")
        .prompt()
        .map_err(|e| BbError::Config(format!("cancelled: {e}")))?;
    Ok(Requested {
        title: Some(clean_title(&title)?),
        description: Some(description.trim_end_matches('\n').to_string()),
    })
}

#[derive(Serialize)]
struct EditRow {
    id: u64,
    title: String,
    description: String,
    url: String,
    changed: Vec<&'static str>,
}

impl EditRow {
    fn from(pr: &PullRequest, changed: Vec<&'static str>) -> Self {
        Self {
            id: pr.id,
            title: pr.title.clone().unwrap_or_default(),
            description: pr.description_text().to_string(),
            url: pr.html_url().to_string(),
            changed,
        }
    }
}

pub async fn run(ctx: &Ctx, args: EditArgs) -> Result<()> {
    let id = args.id;
    let flags = from_flags(&args)?;
    if flags.is_none() && !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
        return Err(BbError::Config(format!(
            "nothing to change on #{id} — pass --title, --description or --description-stdin"
        )));
    }

    let path = ctx.path(&format!("/pullrequests/{id}"));
    let pr: PullRequest = ctx.client.get_json(&path).await?;

    // A closed pull request answers the `PUT` with an unhelpful 400, so the
    // state is checked here where the message can name what is wrong.
    let state = pr.state.as_deref().unwrap_or("UNKNOWN");
    if !state.eq_ignore_ascii_case("OPEN") {
        return Err(BbError::Config(format!(
            "pull request #{id} is {state}, and only an open one can be edited"
        )));
    }

    let current_title = pr.title.clone().unwrap_or_default();
    let current_description = pr.description_text().to_string();
    let requested = match flags {
        Some(requested) => requested,
        None => prompt(&current_title, &current_description)?,
    };
    let plan = plan(&current_title, &current_description, requested);
    let changed = plan.changed();

    if changed.is_empty() {
        match ctx.format {
            Format::Json => output::print_json(&EditRow::from(&pr, changed))?,
            Format::Human => output::info(&format!(
                "pull request #{id} already says that — nothing to do"
            )),
        }
        return Ok(());
    }

    let mut body = serde_json::json!({
        "title": plan.title.as_deref().unwrap_or(&current_title),
    });
    if let Some(description) = &plan.description {
        body["description"] = serde_json::Value::String(description.clone());
    }
    let updated: PullRequest = ctx.client.put_json(&path, &body).await?;

    let row = EditRow::from(&updated, changed);
    match ctx.format {
        Format::Json => output::print_json(&row)?,
        Format::Human => {
            output::success(&format!(
                "pull request #{id} updated: {}",
                row.changed.join(", ")
            ));
            output::info(&row.url);
        }
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn requested(title: Option<&str>, description: Option<&str>) -> Requested {
        Requested {
            title: title.map(str::to_string),
            description: description.map(str::to_string),
        }
    }

    #[test]
    fn a_trailing_newline_alone_is_not_a_description_change() {
        let p = plan("T", "body", requested(None, Some("body\n")));
        assert!(p.changed().is_empty(), "got {p:?}");
    }

    #[test]
    fn clearing_the_description_is_a_change() {
        let p = plan("T", "body", requested(None, Some("")));
        assert_eq!(p.description.as_deref(), Some(""));
    }

    #[test]
    fn an_identical_title_is_dropped_and_a_new_one_kept() {
        assert!(plan("T", "", requested(Some("T"), None)).title.is_none());
        assert_eq!(
            plan("T", "", requested(Some("U"), None)).title.as_deref(),
            Some("U")
        );
    }

    #[test]
    fn changed_lists_title_before_description() {
        let p = plan("T", "a", requested(Some("U"), Some("b")));
        assert_eq!(p.changed(), vec!["title", "description"]);
    }

    #[test]
    fn a_blank_title_is_rejected() {
        assert!(clean_title("   ").is_err());
        assert_eq!(clean_title("  Fix it ").unwrap(), "Fix it");
    }
}
