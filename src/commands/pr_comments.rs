use crate::api::models::{Comment, PullRequest};
use crate::commands::pr::{self, Ctx};
use crate::error::{BbError, Result};
use crate::output::{self, Format};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct CommentView {
    pub id: u64,
    pub author: String,
    pub timestamp: String,
    pub body: String,
    pub file: Option<String>,
    pub line: Option<u64>,
    pub resolved: bool,
    pub parent: Option<u64>,
}

fn to_view(comment: &Comment) -> CommentView {
    let inline = comment.inline.as_ref();
    CommentView {
        id: comment.id,
        author: comment.author().to_string(),
        timestamp: comment
            .created_on
            .as_deref()
            .map(output::relative_time)
            .unwrap_or_else(|| "-".into()),
        body: comment.body(),
        file: inline.and_then(|i| i.path.clone()),
        line: inline.and_then(|i| i.to.or(i.from)),
        resolved: comment.is_resolved(),
        parent: comment.parent_id(),
    }
}

/// Splits comments into general and inline buckets, oldest first. The
/// `unresolved` filter applies only to inline threads — general comments are
/// not resolvable in the Bitbucket API and are always kept.
pub fn partition(
    mut comments: Vec<Comment>,
    unresolved: bool,
) -> (Vec<CommentView>, Vec<CommentView>) {
    comments.sort_by(|a, b| a.created_on.cmp(&b.created_on));

    let mut general = Vec::new();
    let mut inline = Vec::new();
    for comment in &comments {
        if comment.is_inline() {
            if unresolved && comment.is_resolved() {
                continue;
            }
            inline.push(to_view(comment));
        } else {
            general.push(to_view(comment));
        }
    }
    (general, inline)
}

pub async fn view(ctx: &Ctx, id: u64, unresolved: bool, comments_only: bool) -> Result<()> {
    let pr: Option<PullRequest> = if comments_only {
        None
    } else {
        Some(
            ctx.client
                .get_json(&ctx.path(&format!("/pullrequests/{id}")))
                .await?,
        )
    };

    let spinner = output::spinner("fetching comments");
    let comments: Vec<Comment> = ctx
        .client
        .paginate(&ctx.path(&format!("/pullrequests/{id}/comments?pagelen=100")))
        .await?;
    spinner.finish_and_clear();

    let (general, inline) = partition(comments, unresolved);

    match ctx.format {
        Format::Json => output::print_json(&serde_json::json!({
            "pull_request": pr.as_ref().map(|pr| serde_json::json!({
                "id": pr.id,
                "title": pr.title,
                "state": pr.state,
                "author": pr.author_name(),
                "source": pr.source_branch(),
                "destination": pr.destination_branch(),
                "url": pr.html_url(),
            })),
            "general": general,
            "inline": inline,
        }))?,
        Format::Human => {
            if let Some(pr) = &pr {
                output::heading(&format!(
                    "#{} {}",
                    pr.id,
                    pr.title.clone().unwrap_or_default()
                ));
                output::info(&format!(
                    "{} → {} · {} · by {}",
                    pr.source_branch(),
                    pr.destination_branch(),
                    pr.state.clone().unwrap_or_else(|| "-".into()),
                    pr.author_name()
                ));
                output::info(pr.html_url());
                println!();
            }

            output::heading("general comments");
            if general.is_empty() {
                output::info("none");
            }
            for c in &general {
                println!("  {} ({}):", c.author, c.timestamp);
                for line in c.body.lines() {
                    println!("    {line}");
                }
                println!();
            }

            output::heading(if unresolved {
                "inline comments (unresolved)"
            } else {
                "inline comments"
            });
            if inline.is_empty() {
                output::info("none");
            }
            for c in &inline {
                let location = match (c.file.as_deref(), c.line) {
                    (Some(file), Some(line)) => format!("{file}:{line}"),
                    (Some(file), None) => file.to_string(),
                    _ => "-".to_string(),
                };
                let marker = if c.resolved { " [resolved]" } else { "" };
                match c.parent {
                    Some(parent) => println!(
                        "  {location}{marker}  (comment {} · reply to {parent})",
                        c.id
                    ),
                    None => println!("  {location}{marker}  (comment {})", c.id),
                }
                println!("  {} ({}):", c.author, c.timestamp);
                for line in c.body.lines() {
                    println!("    {line}");
                }
                println!();
            }
        }
    }

    Ok(())
}

#[derive(Debug, Default)]
pub struct CommentArgs {
    pub id: u64,
    pub body: Option<String>,
    pub body_stdin: bool,
    pub file: Option<String>,
    pub line: Option<u64>,
    pub reply_to: Option<u64>,
    pub web: bool,
}

/// Builds the request body, rejecting flag combinations the API cannot honour.
pub fn build_payload(args: &CommentArgs, body: &str) -> Result<serde_json::Value> {
    if body.trim().is_empty() {
        return Err(BbError::Config("comment body is empty".into()));
    }
    if args.line.is_some() && args.file.is_none() {
        return Err(BbError::Config("--line requires --file".into()));
    }
    if args.reply_to.is_some() && (args.file.is_some() || args.line.is_some()) {
        return Err(BbError::Config(
            "--reply-to cannot be combined with --file or --line — a reply inherits its parent's location".into(),
        ));
    }

    let mut payload = serde_json::json!({ "content": { "raw": body } });

    if let Some(file) = &args.file {
        let mut inline = serde_json::json!({ "path": file });
        if let Some(line) = args.line {
            inline["to"] = serde_json::Value::from(line);
        }
        payload["inline"] = inline;
    }

    if let Some(parent) = args.reply_to {
        payload["parent"] = serde_json::json!({ "id": parent });
    }

    Ok(payload)
}

fn read_body(args: &CommentArgs) -> Result<String> {
    if args.body_stdin {
        let mut buf = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut buf)?;
        return Ok(buf.trim_end_matches('\n').to_string());
    }
    if let Some(body) = &args.body {
        return Ok(body.clone());
    }
    if !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
        return Err(BbError::Config(
            "no comment body — pass --body or --body-stdin".into(),
        ));
    }
    inquire::Editor::new("comment:")
        .prompt()
        .map_err(|e| BbError::Config(format!("cancelled: {e}")))
}

pub async fn comment(ctx: &Ctx, args: CommentArgs) -> Result<()> {
    let body = read_body(&args)?;
    let payload = build_payload(&args, &body)?;

    let spinner = if ctx.format.is_json() {
        None
    } else {
        Some(output::spinner("posting comment"))
    };
    let created: Comment = ctx
        .client
        .post_json(
            &ctx.path(&format!("/pullrequests/{}/comments", args.id)),
            &payload,
        )
        .await?;
    if let Some(spinner) = spinner {
        spinner.finish_and_clear();
    }

    let url = format!(
        "{}/pull-requests/{}#comment-{}",
        ctx.slug.browse_url(),
        args.id,
        created.id
    );

    if args.web {
        let _ = open::that_detached(&url);
    }

    match ctx.format {
        Format::Json => output::print_json(&serde_json::json!({
            "id": created.id,
            "pull_request": args.id,
            "url": url,
        }))?,
        Format::Human => {
            output::success(&format!("comment {} added to #{}", created.id, args.id));
            output::info(&url);
        }
    }

    Ok(())
}

/// Marks a comment thread as resolved. Bitbucket resolves the whole thread, so
/// `comment` is the id of its root — the entry `bb pr view` reports without a
/// `parent`. The response body carries only the resolution, which adds nothing
/// the caller does not already know, so it is discarded.
pub async fn resolve(ctx: &Ctx, id: u64, comment: u64) -> Result<()> {
    ctx.client
        .post_empty(&resolve_path(ctx, id, comment))
        .await?;
    pr::report(
        ctx,
        &format!("comment {comment} resolved on #{id}"),
        serde_json::json!({ "resolved": comment, "pull_request": id }),
    )
}

/// Reopens a resolved thread.
pub async fn unresolve(ctx: &Ctx, id: u64, comment: u64) -> Result<()> {
    ctx.client.delete(&resolve_path(ctx, id, comment)).await?;
    pr::report(
        ctx,
        &format!("comment {comment} reopened on #{id}"),
        serde_json::json!({ "unresolved": comment, "pull_request": id }),
    )
}

fn resolve_path(ctx: &Ctx, id: u64, comment: u64) -> String {
    ctx.path(&format!("/pullrequests/{id}/comments/{comment}/resolve"))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod build_payload_tests {
    use super::*;

    fn args() -> CommentArgs {
        CommentArgs {
            id: 7,
            ..Default::default()
        }
    }

    #[test]
    fn general_comment_payload_has_only_content() {
        let payload = build_payload(&args(), "hi").unwrap();
        assert_eq!(payload["content"]["raw"], "hi");
        assert!(payload.get("inline").is_none());
        assert!(payload.get("parent").is_none());
    }

    #[test]
    fn inline_payload_carries_path_and_line() {
        let a = CommentArgs {
            file: Some("src/main.rs".into()),
            line: Some(9),
            ..args()
        };
        let payload = build_payload(&a, "hi").unwrap();
        assert_eq!(payload["inline"]["path"], "src/main.rs");
        assert_eq!(payload["inline"]["to"], 9);
    }

    #[test]
    fn file_without_line_comments_on_the_file() {
        let a = CommentArgs {
            file: Some("src/main.rs".into()),
            ..args()
        };
        let payload = build_payload(&a, "hi").unwrap();
        assert_eq!(payload["inline"]["path"], "src/main.rs");
        assert!(payload["inline"].get("to").is_none());
    }

    #[test]
    fn empty_body_rejected() {
        assert!(build_payload(&args(), "   ").is_err());
    }

    #[test]
    fn line_without_file_rejected() {
        let a = CommentArgs {
            line: Some(9),
            ..args()
        };
        assert!(build_payload(&a, "hi").is_err());
    }

    #[test]
    fn reply_with_inline_location_rejected() {
        let a = CommentArgs {
            reply_to: Some(1),
            file: Some("x".into()),
            ..args()
        };
        assert!(build_payload(&a, "hi").is_err());
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod partition_tests {
    use super::*;

    fn comment(json: serde_json::Value) -> Comment {
        serde_json::from_value(json).unwrap()
    }

    #[test]
    fn general_comment_survives_unresolved_filter() {
        let c = comment(serde_json::json!({
            "id": 1,
            "content": { "raw": "hi" },
            "user": { "display_name": "Me" },
            "created_on": "2026-08-04T10:00:00+00:00",
        }));
        let (general, inline) = partition(vec![c], true);
        assert_eq!(general.len(), 1);
        assert!(inline.is_empty());
    }

    fn resolved_inline() -> serde_json::Value {
        serde_json::json!({
            "id": 2,
            "content": { "raw": "fix this" },
            "user": { "display_name": "Me" },
            "created_on": "2026-08-04T10:00:00+00:00",
            "inline": { "path": "src/main.rs", "to": 1 },
            "resolution": { "user": { "display_name": "Me" } },
        })
    }

    #[test]
    fn resolved_inline_comment_dropped_when_unresolved_true() {
        let (_, inline) = partition(vec![comment(resolved_inline())], true);
        assert!(inline.is_empty());
    }

    #[test]
    fn resolved_inline_comment_kept_when_unresolved_false() {
        let (_, inline) = partition(vec![comment(resolved_inline())], false);
        assert_eq!(inline.len(), 1);
    }

    #[test]
    fn buckets_are_oldest_first_by_created_on() {
        let newer = comment(serde_json::json!({
            "id": 1,
            "content": { "raw": "newer" },
            "user": { "display_name": "Me" },
            "created_on": "2026-08-04T12:00:00+00:00",
        }));
        let older = comment(serde_json::json!({
            "id": 2,
            "content": { "raw": "older" },
            "user": { "display_name": "Me" },
            "created_on": "2026-08-04T09:00:00+00:00",
        }));
        let (general, _) = partition(vec![newer, older], false);
        assert_eq!(general[0].body, "older");
        assert_eq!(general[1].body, "newer");
    }

    #[test]
    fn empty_resolution_object_counts_as_resolved() {
        let c = comment(serde_json::json!({
            "id": 3,
            "content": { "raw": "done" },
            "user": { "display_name": "Me" },
            "created_on": "2026-08-04T10:00:00+00:00",
            "inline": { "path": "src/main.rs", "to": 1 },
            "resolution": {},
        }));
        let (_, inline) = partition(vec![c], true);
        assert!(inline.is_empty());
    }
}
