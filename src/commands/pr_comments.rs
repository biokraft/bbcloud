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

/// `file:line`, or the bare file when the comment sits on no single line.
/// `None` when the comment is not inline, leaving the fallback to the caller.
fn location(file: Option<&str>, line: Option<u64>) -> Option<String> {
    match (file, line) {
        (Some(file), Some(line)) => Some(format!("{file}:{line}")),
        (Some(file), None) => Some(file.to_string()),
        _ => None,
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
                let location =
                    location(c.file.as_deref(), c.line).unwrap_or_else(|| "-".to_string());
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
///
/// Resolving hides a reviewer's point from the pull request, and nothing in the
/// api asks whether that point was actually addressed. So a human does: the
/// command confirms first, and `yes` is the only way past it.
pub async fn resolve(ctx: &Ctx, id: u64, comment: u64, yes: bool) -> Result<()> {
    if !yes {
        approve(ctx, id, comment).await?;
    }
    ctx.client
        .post_empty(&resolve_path(ctx, id, comment))
        .await?;
    pr::report(
        ctx,
        &format!("comment {comment} resolved on #{id}"),
        serde_json::json!({ "resolved": comment, "pull_request": id }),
    )
}

/// Puts the thread in front of a human and waits for a yes. The prompt renders
/// on stderr, so `--json` stdout stays pure.
///
/// With no terminal there is nobody to ask, so this names the flag rather than
/// blocking on input that will not arrive. That also means an agent or a CI job
/// cannot resolve anything unless whoever wrote the command line said `--yes`.
async fn approve(ctx: &Ctx, id: u64, comment: u64) -> Result<()> {
    if !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
        return Err(BbError::Config(
            "resolving needs approval — answer the prompt in a terminal, or pass --yes to approve up front".into(),
        ));
    }
    gate(ctx, id, comment, ask_human).await
}

/// Shows the thread, then turns the answer into a verdict. `ask` is a parameter
/// because the real prompt needs a terminal no test has: this way the parts that
/// carry the decision are exercised, and `ask_human` is left holding nothing but
/// the rendering.
async fn gate<A>(ctx: &Ctx, id: u64, comment: u64, ask: A) -> Result<()>
where
    A: FnOnce(&str) -> Result<bool>,
{
    // Fetched only on this path: `--yes` must cost no extra request.
    let thread: Comment = ctx
        .client
        .get_json(&ctx.path(&format!("/pullrequests/{id}/comments/{comment}")))
        .await?;
    let where_ = resolvable(&thread)?;

    if ask(&format!("resolve {}?", describe(&thread, &where_)))? {
        Ok(())
    } else {
        // Declining is an error, not a quiet success: a script reading exit 0 as
        // "resolved" must never see one.
        Err(BbError::Config(format!("comment {comment} left open")))
    }
}

/// Left uncovered on purpose: it needs a terminal, and it holds no decision that
/// a test could get wrong — the same shape as the `inquire::Editor` call in
/// `read_body`.
fn ask_human(question: &str) -> Result<bool> {
    inquire::Confirm::new(question)
        .with_default(false)
        .prompt()
        .map_err(|e| BbError::Config(format!("cancelled: {e}")))
}

/// Rejects the ids the endpoint cannot act on, and yields where the thread sits.
///
/// Bitbucket answers 403 both for a reply and for a comment that is not on the
/// diff, and the generic 403 text blames the token's scopes — a wrong diagnosis
/// that costs the reader real time. So these two cases are named here instead,
/// before anything is sent or any human is asked to approve a doomed request.
fn resolvable(thread: &Comment) -> Result<String> {
    if let Some(root) = thread.parent_id() {
        return Err(BbError::Config(format!(
            "comment {} is a reply — resolve the thread's first comment, {root}",
            thread.id
        )));
    }
    let inline = thread.inline.as_ref();
    location(
        inline.and_then(|i| i.path.as_deref()),
        inline.and_then(|i| i.to.or(i.from)),
    )
    .ok_or_else(|| {
        BbError::Config(format!(
            "comment {} is not on the diff, and bitbucket resolves only inline threads",
            thread.id
        ))
    })
}

/// One line naming what the approval covers: where the thread sits, who raised
/// it, and what it says.
fn describe(thread: &Comment, where_: &str) -> String {
    format!(
        "the thread at {where_} by {} — \"{}\"",
        thread.author(),
        summarize(&thread.body())
    )
}

/// First line of a comment, short enough to sit inside a prompt.
fn summarize(body: &str) -> String {
    const MAX: usize = 72;
    let first = body.lines().next().unwrap_or_default().trim();
    if first.chars().count() <= MAX {
        return first.to_string();
    }
    format!("{}…", first.chars().take(MAX).collect::<String>())
}

/// Reopens a resolved thread. This is not gated: it restores a reviewer's point
/// rather than hiding one, so the worst case is noise a human can see.
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

/// Everything the gate decides, with the prompt answered by the test instead of
/// by a human. The one thing left uncovered is `ask_human`, which needs a
/// terminal and holds no decision of its own.
#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod gate_tests {
    use super::*;
    use crate::api::Client;
    use crate::credentials::Credentials;
    use crate::repo::RepoSlug;
    use crate::secret::SecretString;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn ctx(server: &MockServer) -> Ctx {
        Ctx {
            client: Client::new(
                Credentials {
                    email: "dev@example.com".into(),
                    token: SecretString::from("t0ken-value"),
                },
                server.uri(),
            )
            .unwrap(),
            slug: RepoSlug::parse("acme/widgets").unwrap(),
            format: Format::Human,
        }
    }

    async fn mount_thread(server: &MockServer) {
        Mock::given(method("GET"))
            .and(path(
                "/repositories/acme/widgets/pullrequests/7/comments/900",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": 900,
                "content": { "raw": "this drops the error" },
                "user": { "display_name": "Reviewer" },
                "inline": { "path": "src/auth.rs", "to": 88 },
            })))
            .expect(1)
            .mount(server)
            .await;
    }

    #[tokio::test]
    async fn a_yes_passes_the_gate_and_the_question_names_the_thread() {
        let server = MockServer::start().await;
        mount_thread(&server).await;

        let mut asked = String::new();
        let result = gate(&ctx(&server), 7, 900, |question| {
            asked = question.to_string();
            Ok(true)
        })
        .await;

        assert!(result.is_ok(), "{result:?}");
        assert!(asked.starts_with("resolve "), "{asked}");
        assert!(asked.contains("src/auth.rs:88"), "{asked}");
        assert!(asked.contains("this drops the error"), "{asked}");
    }

    #[tokio::test]
    async fn a_no_fails_and_names_the_comment_left_open() {
        let server = MockServer::start().await;
        mount_thread(&server).await;

        let err = gate(&ctx(&server), 7, 900, |_| Ok(false))
            .await
            .unwrap_err();

        let shown = err.to_string();
        assert!(shown.contains("900"), "{shown}");
        assert!(shown.contains("left open"), "{shown}");
        assert_eq!(err.exit_code(), 1);
    }

    #[tokio::test]
    async fn a_cancelled_prompt_stops_the_gate() {
        let server = MockServer::start().await;
        mount_thread(&server).await;

        let result = gate(&ctx(&server), 7, 900, |_| {
            Err(BbError::Config("cancelled: interrupted".into()))
        })
        .await;

        assert!(result.is_err(), "an unanswered prompt must not resolve");
    }

    /// A reply id is refused before the prompt: the request it would send is the
    /// one bitbucket answers 403 for.
    #[tokio::test]
    async fn a_reply_never_reaches_the_prompt() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(
                "/repositories/acme/widgets/pullrequests/7/comments/901",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": 901,
                "content": { "raw": "fixed" },
                "user": { "display_name": "Me" },
                "inline": { "path": "src/auth.rs", "to": 88 },
                "parent": { "id": 900 },
            })))
            .mount(&server)
            .await;

        let err = gate(&ctx(&server), 7, 901, |_| unreachable!("asked anyway"))
            .await
            .unwrap_err();

        assert!(err.to_string().contains("900"), "{err}");
    }

    /// A bad id fails at the lookup, so nobody is asked to approve a thread that
    /// does not exist.
    #[tokio::test]
    async fn an_unknown_comment_never_reaches_the_prompt() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(
                "/repositories/acme/widgets/pullrequests/7/comments/404",
            ))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let err = gate(&ctx(&server), 7, 404, |_| unreachable!("asked anyway"))
            .await
            .unwrap_err();

        assert_eq!(err.exit_code(), 3);
    }
}

/// The confirmation prompt cannot be driven from a piped stdin — that is what
/// makes it a gate — so what a human is asked is asserted here instead.
#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod describe_tests {
    use super::*;

    fn comment(json: serde_json::Value) -> Comment {
        serde_json::from_value(json).unwrap()
    }

    fn inline_thread() -> serde_json::Value {
        serde_json::json!({
            "id": 600,
            "content": { "raw": "this drops the error\nsecond line" },
            "user": { "display_name": "Reviewer" },
            "inline": { "path": "src/auth.rs", "to": 88 },
        })
    }

    #[test]
    fn names_the_place_the_author_and_the_point() {
        let thread = comment(inline_thread());
        let shown = describe(&thread, &resolvable(&thread).unwrap());
        assert!(shown.contains("src/auth.rs:88"), "{shown}");
        assert!(shown.contains("Reviewer"), "{shown}");
        assert!(shown.contains("this drops the error"), "{shown}");
        assert!(!shown.contains("second line"), "one line only: {shown}");
    }

    /// Bitbucket answers 403 for a reply, so the id is refused with the root to
    /// use instead — a prompt claiming to resolve that root would be a lie.
    #[test]
    fn a_reply_is_refused_and_names_the_root() {
        let mut json = inline_thread();
        json["id"] = serde_json::Value::from(601);
        json["parent"] = serde_json::json!({ "id": 600 });
        let err = resolvable(&comment(json)).unwrap_err().to_string();
        assert!(err.contains("reply"), "{err}");
        assert!(err.contains("600"), "must name the root: {err}");
    }

    /// "Not on the diff" is the other documented 403: a general comment has no
    /// thread to resolve.
    #[test]
    fn a_general_comment_is_refused() {
        let err = resolvable(&comment(serde_json::json!({
            "id": 42,
            "content": { "raw": "a general remark" },
            "user": { "display_name": "Reviewer" },
        })))
        .unwrap_err()
        .to_string();
        assert!(err.contains("42"), "{err}");
        assert!(err.contains("inline"), "{err}");
    }

    /// An inline root with a file but no line is still resolvable.
    #[test]
    fn a_whole_file_thread_is_resolvable() {
        let where_ = resolvable(&comment(serde_json::json!({
            "id": 500,
            "content": { "raw": "whole-file note" },
            "user": { "display_name": "Reviewer" },
            "inline": { "path": "src/lib.rs" },
        })))
        .unwrap();
        assert_eq!(where_, "src/lib.rs");
    }

    #[test]
    fn a_long_point_is_truncated_on_a_char_boundary() {
        let body = "ü".repeat(200);
        let shown = summarize(&body);
        assert!(shown.ends_with('…'), "{shown}");
        assert_eq!(shown.chars().count(), 73);
    }

    #[test]
    fn location_is_none_when_the_comment_is_not_inline() {
        assert_eq!(location(None, Some(9)), None);
        assert_eq!(location(Some("a.rs"), None).unwrap(), "a.rs");
        assert_eq!(location(Some("a.rs"), Some(9)).unwrap(), "a.rs:9");
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
