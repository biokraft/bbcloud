use crate::api::models::{
    Commit, DiffStatEntry, EffectiveReviewer, PullRequest, ReviewerRef, User,
};
use crate::api::{repo_path, Client};
use crate::credentials;
use crate::error::{BbError, Result};
use crate::git;
use crate::output::{self, Format};
use crate::repo::{self, RepoSlug};
use crate::users::{self, UserPool};
use serde::Serialize;

pub struct Ctx {
    pub client: Client,
    pub slug: RepoSlug,
    pub format: Format,
}

impl Ctx {
    pub fn new(repo: Option<&str>, format: Format) -> Result<Self> {
        let creds = credentials::load()?;
        let slug = repo::resolve(repo)?;
        let client = Client::from_env(creds)?;
        Ok(Self {
            client,
            slug,
            format,
        })
    }

    pub fn path(&self, suffix: &str) -> String {
        repo_path(&self.slug, suffix)
    }
}

pub async fn diff(ctx: &Ctx, id: u64) -> Result<()> {
    let text = ctx
        .client
        .get_text(&ctx.path(&format!("/pullrequests/{id}/diff")))
        .await?;
    if ctx.format.is_json() {
        output::print_json(&serde_json::json!({ "id": id, "diff": text }))?;
    } else {
        print!("{text}");
    }
    Ok(())
}

pub async fn files(ctx: &Ctx, id: u64) -> Result<()> {
    let entries: Vec<DiffStatEntry> = ctx
        .client
        .paginate(&ctx.path(&format!("/pullrequests/{id}/diffstat?pagelen=100")))
        .await?;

    #[derive(Serialize)]
    struct FileRow {
        status: String,
        path: String,
    }

    let rows: Vec<FileRow> = entries
        .iter()
        .map(|e| FileRow {
            status: e.status.clone().unwrap_or_else(|| "-".into()),
            path: e.path().to_string(),
        })
        .collect();

    match ctx.format {
        Format::Json => output::print_json(&rows)?,
        Format::Human => output::print_table(
            &["STATUS", "PATH"],
            rows.iter()
                .map(|r| vec![r.status.clone(), r.path.clone()])
                .collect(),
        ),
    }
    Ok(())
}

pub async fn commits(ctx: &Ctx, id: u64) -> Result<()> {
    let commits: Vec<Commit> = ctx
        .client
        .paginate(&ctx.path(&format!("/pullrequests/{id}/commits?pagelen=100")))
        .await?;

    #[derive(Serialize)]
    struct CommitRow {
        hash: String,
        summary: String,
    }

    let rows: Vec<CommitRow> = commits
        .iter()
        .map(|c| CommitRow {
            hash: c.hash.clone().unwrap_or_default().chars().take(7).collect(),
            summary: c
                .summary
                .as_ref()
                .and_then(|s| s.raw.clone())
                .unwrap_or_default()
                .lines()
                .next()
                .unwrap_or("")
                .to_string(),
        })
        .collect();

    match ctx.format {
        Format::Json => output::print_json(&rows)?,
        Format::Human => output::print_table(
            &["HASH", "SUMMARY"],
            rows.iter()
                .map(|r| vec![r.hash.clone(), r.summary.clone()])
                .collect(),
        ),
    }
    Ok(())
}

/// Asks the author to change something. Marking a pull request is a claim about
/// someone else's work that the api cannot tell was warranted, so a human says
/// yes: the command confirms first, and `--yes` is the only way past it.
pub async fn request_changes(ctx: &Ctx, id: u64, yes: bool) -> Result<()> {
    if !yes {
        gate(
            ctx,
            id,
            "request changes on",
            "requesting changes",
            ask_human,
        )
        .await?;
    }
    ctx.client
        .post_empty(&ctx.path(&format!("/pullrequests/{id}/request-changes")))
        .await?;
    report(
        ctx,
        &format!("changes requested on #{id}"),
        serde_json::json!({ "requested_changes": id }),
    )
}

/// Withdraws a change request. Gated for the same reason as its opposite, from
/// the other side: withdrawing clears a block on a merge.
pub async fn unrequest_changes(ctx: &Ctx, id: u64, yes: bool) -> Result<()> {
    if !yes {
        gate(
            ctx,
            id,
            "withdraw the change request on",
            "withdrawing a change request",
            ask_human,
        )
        .await?;
    }
    ctx.client
        .delete(&ctx.path(&format!("/pullrequests/{id}/request-changes")))
        .await?;
    report(
        ctx,
        &format!("change request removed from #{id}"),
        serde_json::json!({ "unrequested_changes": id }),
    )
}

/// Puts the pull request in front of a human and waits for a yes.
///
/// With no terminal there is nobody to ask, so this names the flag rather than
/// blocking on input that will not arrive. That also means an agent or a CI job
/// cannot mark anything unless whoever wrote the command line said `--yes`.
///
/// The pull request is fetched only on this path: `--yes` must cost no extra
/// request.
///
/// `verb` opens the question a human answers; `action` names the same thing as a
/// noun, for the error a caller with no terminal gets instead. Two forms rather
/// than one because a verb phrase reads wrong as a sentence's subject, and that
/// error is the only thing an agent or a CI job ever sees.
async fn gate<A>(ctx: &Ctx, id: u64, verb: &str, action: &str, ask: A) -> Result<()>
where
    A: FnOnce(&str) -> Result<bool>,
{
    if !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
        return Err(BbError::Config(format!(
            "{action} on #{id} needs approval — answer the prompt in a terminal, or pass --yes to approve up front"
        )));
    }
    let pr: PullRequest = ctx
        .client
        .get_json(&ctx.path(&format!("/pullrequests/{id}")))
        .await?;
    decide(id, &prompt_line(verb, &pr), ask)
}

/// Renders the question. Kept separate so a test can assert what a human is
/// shown without needing a terminal or a server.
fn prompt_line(verb: &str, pr: &PullRequest) -> String {
    let title = pr.title.as_deref().unwrap_or("untitled");
    let author = pr.author.as_ref().map(|a| a.name()).unwrap_or("someone");
    format!("{verb} #{} \"{title}\" by {author}?", pr.id)
}

/// Turns the answer into a verdict. `ask` is a parameter because the real prompt
/// needs a terminal no test has: this way the part that carries the decision is
/// exercised, and `ask_human` is left holding nothing but the rendering.
fn decide<A>(id: u64, question: &str, ask: A) -> Result<()>
where
    A: FnOnce(&str) -> Result<bool>,
{
    if ask(question)? {
        Ok(())
    } else {
        // Declining is an error, not a quiet success: a script reading exit 0 as
        // "marked" must never see one. The human just read the question, so the
        // message states the outcome rather than echoing it back at them.
        Err(BbError::Config(format!("#{id} left unchanged")))
    }
}

/// Left uncovered on purpose: it needs a terminal, and it holds no decision that
/// a test could get wrong.
fn ask_human(question: &str) -> Result<bool> {
    inquire::Confirm::new(question)
        .with_default(false)
        .prompt()
        .map_err(|e| BbError::Config(format!("cancelled: {e}")))
}

pub fn report(ctx: &Ctx, human: &str, json: serde_json::Value) -> Result<()> {
    match ctx.format {
        Format::Json => output::print_json(&json),
        Format::Human => {
            output::success(human);
            Ok(())
        }
    }
}

#[derive(Debug, Default)]
pub struct CreateArgs {
    pub target: String,
    pub source: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
    pub description_stdin: bool,
    pub no_default_reviewers: bool,
    pub reviewer: Option<String>,
    pub interactive: bool,
    pub web: bool,
    pub close_source_branch: bool,
}

async fn default_reviewers(ctx: &Ctx) -> Result<Vec<ReviewerRef>> {
    let me: User = ctx.client.get_json("/user").await?;
    let my_uuid = me.uuid.unwrap_or_default();
    let reviewers: Vec<EffectiveReviewer> = ctx
        .client
        .paginate(&ctx.path("/effective-default-reviewers?pagelen=100"))
        .await?;
    let mut seen = std::collections::HashSet::new();
    Ok(reviewers
        .into_iter()
        .filter_map(|reviewer| reviewer.user)
        .filter_map(|user| user.uuid)
        .filter(|uuid| *uuid != my_uuid)
        .filter(|uuid| seen.insert(uuid.clone()))
        .map(|uuid| ReviewerRef { uuid })
        .collect())
}

/// Resolves an explicit `--reviewer` list to uuids, dropping the author because
/// bitbucket answers 400 when the author is tagged as a reviewer. Every name is
/// resolved before the caller opens anything, so one bad name creates no pull
/// request rather than one with a reviewer set nobody chose.
async fn named_reviewers(ctx: &Ctx, names: &str) -> Result<Vec<ReviewerRef>> {
    let requested: Vec<&str> = names
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if requested.is_empty() {
        return Err(BbError::Config("no reviewer name given".into()));
    }

    // Split first, then resolve: a `{uuid}` is already exact, so the pool is
    // loaded only when there is at least one name that actually needs it.
    let mut uuids: Vec<String> = Vec::new();
    let mut names: Vec<&str> = Vec::new();
    for name in requested {
        match users::uuid_user(name) {
            // `uuid_user` only answers for a `{uuid}`, so it always has one.
            Some(user) => {
                let uuid = user.uuid.unwrap_or_default();
                if !uuid.is_empty() && !uuids.contains(&uuid) {
                    uuids.push(uuid);
                }
            }
            None => names.push(name),
        }
    }
    if !names.is_empty() {
        let pool = UserPool::load(&ctx.client, &ctx.slug).await?;
        for name in names {
            let user = pool.resolve_for_write(name, &[])?;
            let uuid = user
                .uuid
                .clone()
                .ok_or_else(|| BbError::Config(format!("`{}` has no uuid to tag", user.name())))?;
            if !uuids.contains(&uuid) {
                uuids.push(uuid);
            }
        }
    }

    let me: User = ctx.client.get_json("/user").await?;
    let my_uuid = me.uuid.unwrap_or_default();
    uuids.retain(|uuid| *uuid != my_uuid);

    Ok(uuids.into_iter().map(|uuid| ReviewerRef { uuid }).collect())
}

/// Normalizes a description read from stdin.
///
/// Whether the input is a pipe or a live terminal is decided by the caller and
/// passed in, so the rule is a pure function and can be tested without a tty —
/// a terminal here would block forever waiting for an EOF nobody will type.
fn description_from(body: &str, interactive: bool) -> Result<String> {
    if interactive {
        return Err(BbError::Config(
            "--description-stdin requires piped or redirected input".into(),
        ));
    }
    // A description authored on Windows arrives with CRLF, and the trailing \r
    // would otherwise be sent to bitbucket as part of the last line.
    Ok(body
        .replace("\r\n", "\n")
        .trim_end_matches('\n')
        .to_string())
}

/// Reads the body, given a verdict on where it is coming from.
///
/// The reader and the verdict are both parameters so that a live terminal is a
/// case this can be asked about, rather than one only a real tty can produce.
fn description_from_reader(reader: &mut impl std::io::Read, interactive: bool) -> Result<String> {
    // A terminal is refused before anything is read: reading first would block
    // forever waiting for an EOF that only a redirect will ever send.
    if interactive {
        return Err(BbError::Config(
            "--description-stdin requires piped or redirected input".into(),
        ));
    }
    let mut body = String::new();
    reader.read_to_string(&mut body)?;
    description_from(&body, false)
}

fn read_description_from_stdin() -> Result<String> {
    let interactive = std::io::IsTerminal::is_terminal(&std::io::stdin());
    description_from_reader(&mut std::io::stdin(), interactive)
}

pub async fn create(ctx: &Ctx, args: CreateArgs) -> Result<()> {
    if args.description_stdin && args.description.is_some() {
        return Err(BbError::Config(
            "--description and --description-stdin cannot be used together".into(),
        ));
    }
    if args.description_stdin && args.interactive {
        return Err(BbError::Config(
            "--description-stdin cannot be combined with --interactive".into(),
        ));
    }
    let source = match args.source {
        Some(branch) => branch,
        None => git::current_branch()?,
    };

    let mut seen = std::collections::HashSet::new();
    let targets: Vec<String> = args
        .target
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .filter(|s| seen.insert(s.clone()))
        .collect();
    if targets.is_empty() {
        return Err(BbError::Config("no target branch given".into()));
    }
    if targets.contains(&source) {
        return Err(BbError::Config(format!(
            "source and target are both `{source}`"
        )));
    }

    let mut title = args.title;
    let mut description = if args.description_stdin {
        Some(read_description_from_stdin()?)
    } else {
        args.description
    };
    if args.interactive {
        if title.is_none() {
            let entered = inquire::Text::new("title:")
                .with_help_message("leave empty for the default")
                .prompt()
                .map_err(|e| BbError::Config(format!("cancelled: {e}")))?;
            title = Some(entered).filter(|t| !t.trim().is_empty());
        }
        if description.is_none() {
            let entered = inquire::Editor::new("description:")
                .prompt()
                .map_err(|e| BbError::Config(format!("cancelled: {e}")))?;
            description = Some(entered).filter(|t| !t.trim().is_empty());
        }
    }

    // An explicit list is the whole list: naming reviewers means the repository's
    // default set never arrives uninvited, which is the reason to name them.
    let reviewers = match args.reviewer.as_deref() {
        Some(names) => named_reviewers(ctx, names).await?,
        None if args.no_default_reviewers => Vec::new(),
        None => default_reviewers(ctx).await?,
    };

    #[derive(Serialize)]
    struct Created {
        id: u64,
        target: String,
        url: String,
    }

    let mut created = Vec::new();
    for target in targets {
        // NOTE: `title` and `description` are `Option<String>` owned across loop
        // iterations, and `serde_json::json!` moves any value given by value. We
        // borrow `&source`/`&target` and use `.as_deref()` on the options so the
        // macro only ever sees references, leaving the originals intact for the
        // next iteration and for the default-title fallback, success line, and
        // `Created { target, .. }` below (where an owned `target` is genuinely
        // needed, so it is consumed there instead of inside `json!`).
        let default_title = format!("Merge {source} into {target}");
        let body_title = title.as_deref().unwrap_or(&default_title);
        let mut body = serde_json::json!({
            "title": body_title,
            "source": { "branch": { "name": &source } },
            "destination": { "branch": { "name": &target } },
            "reviewers": reviewers,
            "close_source_branch": args.close_source_branch,
        });
        if let Some(text) = description.as_deref() {
            body["description"] = serde_json::Value::String(text.to_string());
        }

        let spinner = output::spinner(&format!("opening {source} \u{2192} {target}"));
        let pr: PullRequest = ctx
            .client
            .post_json(&ctx.path("/pullrequests"), &body)
            .await?;
        spinner.finish_and_clear();

        let url = if pr.html_url() == "-" {
            format!("{}/pull-requests/{}", ctx.slug.browse_url(), pr.id)
        } else {
            pr.html_url().to_string()
        };

        if !ctx.format.is_json() {
            output::success(&format!("#{} {source} \u{2192} {target}", pr.id));
            output::info(&url);
        }
        if args.web {
            let _ = open::that_detached(&url);
        }
        created.push(Created {
            id: pr.id,
            target,
            url,
        });
    }

    if ctx.format.is_json() {
        output::print_json(&created)?;
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn a_yes_lets_the_write_proceed() {
        assert!(decide(42, "request changes on #42?", |_| Ok(true)).is_ok());
    }

    #[test]
    fn a_no_is_an_error_naming_the_pull_request() {
        // The question deliberately carries no id, so the assertion below proves
        // the message is built from the argument rather than echoing the prompt.
        let err = decide(42, "request changes?", |_| Ok(false)).unwrap_err();
        assert!(
            err.to_string().contains("#42"),
            "the error must name the pull request, got: {err}"
        );
    }

    #[test]
    fn the_prompt_line_carries_title_and_author() {
        let pr = PullRequest {
            id: 42,
            title: Some("fix auth token expiry".into()),
            state: None,
            author: Some(User {
                uuid: None,
                account_id: None,
                display_name: Some("Dana".into()),
                nickname: None,
            }),
            source: None,
            destination: None,
            links: None,
            reviewers: Vec::new(),
            participants: Vec::new(),
            draft: false,
            updated_on: None,
            created_on: None,
            comment_count: None,
            task_count: None,
            description: None,
            summary: None,
        };
        let line = prompt_line("request changes on", &pr);
        assert!(line.contains("#42"), "got: {line}");
        assert!(line.contains("fix auth token expiry"), "got: {line}");
        assert!(line.contains("Dana"), "got: {line}");
    }
}

/// clap rejects these flag pairs on the command line, but `create` is public and
/// a library caller can build an `CreateArgs` that trips them. The guards have
/// to hold on their own, and they must hold before the first request goes out.
#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod create_guard_tests {
    use super::*;
    use crate::api::Client;
    use crate::credentials::Credentials;
    use crate::output::Format;
    use crate::secret::SecretString;

    fn args() -> CreateArgs {
        CreateArgs {
            target: "main".into(),
            source: Some("feature/x".into()),
            title: Some("t".into()),
            description: None,
            description_stdin: false,
            no_default_reviewers: false,
            reviewer: None,
            interactive: false,
            web: false,
            close_source_branch: false,
        }
    }

    async fn ctx(server: &wiremock::MockServer) -> Ctx {
        Ctx {
            client: Client::new(
                Credentials {
                    email: "dev@example.com".into(),
                    token: SecretString::from("t0ken-value"),
                },
                server.uri(),
            )
            .unwrap(),
            slug: crate::repo::RepoSlug::parse("acme/widgets").unwrap(),
            format: Format::Json,
        }
    }

    #[tokio::test]
    async fn a_description_cannot_arrive_from_two_places_at_once() {
        let server = wiremock::MockServer::start().await;
        let ctx = ctx(&server).await;
        let err = create(
            &ctx,
            CreateArgs {
                description: Some("typed".into()),
                description_stdin: true,
                ..args()
            },
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("cannot be used together"), "{err}");

        // Piped prose and the interactive editor are two different answers to
        // the same question, and neither can be taken as the other's.
        let err = create(
            &ctx,
            CreateArgs {
                description_stdin: true,
                interactive: true,
                ..args()
            },
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("--interactive"), "{err}");

        assert!(
            server
                .received_requests()
                .await
                .unwrap_or_default()
                .is_empty(),
            "a rejected combination must not have opened a pull request"
        );
    }
}

/// The pipe-or-redirect rule, as a pure function. The decision is the caller's
/// to make because only it can see the terminal; the consequence of that
/// decision is what is worth pinning down.
#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod description_tests {
    use super::*;

    #[test]
    fn a_live_terminal_is_refused_rather_than_left_to_block() {
        let err = description_from("body", true).unwrap_err();
        assert!(err.to_string().contains("piped or redirected"), "{err}");
    }

    /// A reader that records being touched, so the ordering can be asserted
    /// rather than assumed: a terminal must be refused *before* any read, or
    /// it blocks forever waiting for an EOF.
    struct Records<'a> {
        body: &'a [u8],
        touched: std::cell::Cell<bool>,
    }
    impl std::io::Read for Records<'_> {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            self.touched.set(true);
            if self.body.is_empty() {
                return Ok(0);
            }
            let take = self.body.len().min(buf.len());
            buf[..take].copy_from_slice(&self.body[..take]);
            self.body = &self.body[take..];
            Ok(take)
        }
    }

    #[test]
    fn a_terminal_is_refused_before_anything_is_read() {
        let mut reader = Records {
            body: b"never read",
            touched: std::cell::Cell::new(false),
        };
        let err = description_from_reader(&mut reader, true).unwrap_err();
        assert!(err.to_string().contains("piped or redirected"), "{err}");
        assert!(
            !reader.touched.get(),
            "stdin was read before the terminal was refused"
        );
    }

    #[test]
    fn a_pipe_is_read_and_normalized() {
        let mut reader = Records {
            body: b"hello\r\nworld\r\n",
            touched: std::cell::Cell::new(false),
        };
        assert_eq!(
            description_from_reader(&mut reader, false).unwrap(),
            "hello\nworld"
        );
        assert!(reader.touched.get(), "a pipe should have been read");
    }

    #[test]
    fn windows_line_endings_do_not_reach_bitbucket() {
        // A body authored on Windows arrives with CRLF; the trailing \r would
        // otherwise become part of the last line of the description.
        assert_eq!(
            description_from("one\r\ntwo\r\n", false).unwrap(),
            "one\ntwo"
        );
    }

    #[test]
    fn internal_newlines_and_a_missing_trailing_newline_are_kept() {
        assert_eq!(description_from("a\n\nb", false).unwrap(), "a\n\nb");
        assert_eq!(description_from("", false).unwrap(), "");
    }
}
