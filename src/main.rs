#![forbid(unsafe_code)]

use bb_cli::commands;
use bb_cli::error::{BbError, Result};
use bb_cli::output::{self, Format};
use bb_cli::skill;
use bb_cli::update_check;
use bb_cli::workspace;
use clap::{CommandFactory, Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "bb",
    version,
    about = "Bitbucket Cloud CLI",
    propagate_version = true
)]
struct Cli {
    /// Output machine-readable json
    #[arg(long, global = true)]
    json: bool,

    /// Repository to act on, as `workspace/repo` or a bitbucket url
    #[arg(long, short = 'R', global = true, env = "BB_REPO")]
    repo: Option<String>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Manage authentication
    Auth {
        #[command(subcommand)]
        command: AuthCommand,
    },
    /// Work with pull requests
    Pr {
        #[command(subcommand)]
        command: PrCommand,
    },
    /// Work with branches
    Branch {
        #[command(subcommand)]
        command: BranchCommand,
    },
    /// Work with bitbucket projects
    Project {
        #[command(subcommand)]
        command: ProjectCommand,
    },
    /// Work with repositories
    Repo {
        #[command(subcommand)]
        command: RepoCommand,
    },
    /// Open the repository in a browser
    #[command(alias = "b")]
    Browse {
        /// Print the url instead of opening it
        #[arg(long)]
        print: bool,
        /// Open a specific pull request
        #[arg(long, conflicts_with = "branches")]
        pr: Option<u64>,
        /// Open the branches page
        #[arg(long)]
        branches: bool,
    },
    /// Print a shell completion script
    Completions {
        /// bash, zsh, fish, powershell or elvish
        shell: clap_complete::Shell,
    },
    /// Check for a newer release and update this install
    Update,
    /// Install the bundled agent skills so your coding agent can drive `bb`
    ///
    /// `bb skills` is accepted too. The singular is canonical, for consistency
    /// with every other command group, but the plural is what people reach for
    /// — the thing being managed is a set of files, and "skills" is the noun
    /// every agent's own documentation uses.
    #[command(visible_alias = "skills")]
    Skill {
        #[command(subcommand)]
        command: SkillCommand,
    },
}

#[derive(Subcommand)]
enum SkillCommand {
    /// Install or refresh the skill for the agents in this project
    Install {
        /// Which agent layout to write: agents, claude or all (default: auto-detect)
        #[arg(long)]
        agent: Option<String>,
        /// Install into your home directory instead of this project
        #[arg(long)]
        global: bool,
        /// Overwrite a skill file that was edited locally
        #[arg(long)]
        force: bool,
        /// Only act on this skill; omit for all of them
        #[arg(long)]
        skill: Option<String>,
        /// Install every skill without asking
        #[arg(long, conflicts_with = "skill")]
        all: bool,
    },
    /// Show where the skill is installed and whether it is current
    Status,
    /// Remove skills this tool installed
    Uninstall {
        /// Act on your home directory instead of this project
        #[arg(long)]
        global: bool,
        /// Remove a skill file that was edited locally
        #[arg(long)]
        force: bool,
        /// Only act on this skill; omit for all of them
        #[arg(long)]
        skill: Option<String>,
    },
}

#[derive(Subcommand)]
enum BranchCommand {
    /// List branches
    #[command(alias = "l", alias = "ls")]
    List {
        /// Only branches whose last commit author matches this substring
        #[arg(long, short = 'u')]
        user: Option<String>,
        /// Only branches whose name matches this substring
        #[arg(long, short = 'n')]
        name: Option<String>,
        /// Maximum rows to print
        #[arg(long, default_value_t = 100)]
        limit: usize,
    },
}

#[derive(Subcommand)]
enum ProjectCommand {
    /// List the projects in a workspace
    #[command(alias = "l", alias = "ls")]
    List {
        /// Only projects whose key or name matches this substring
        #[arg(long, short = 'n')]
        name: Option<String>,
        /// Maximum rows to print
        #[arg(long, default_value_t = 100)]
        limit: usize,
        /// Workspace to act on; defaults to BB_WORKSPACE, then the git remote
        #[arg(long)]
        workspace: Option<String>,
    },
}

#[derive(Subcommand)]
enum RepoCommand {
    /// Create a repository in a project
    Create {
        /// Repository name, used as the slug
        name: String,
        /// Project to create it in, by key; prompts when omitted in a terminal
        #[arg(long)]
        project: Option<String>,
        /// One-line description
        #[arg(long)]
        description: Option<String>,
        /// Create a public repository; private is the default
        #[arg(long)]
        public: bool,
        /// Workspace to act on; defaults to BB_WORKSPACE, then the git remote
        #[arg(long)]
        workspace: Option<String>,
    },
    /// List the repositories in a workspace
    #[command(alias = "l", alias = "ls")]
    List {
        /// Only repositories in this project, by key
        #[arg(long)]
        project: Option<String>,
        /// Only repositories whose name matches this substring
        #[arg(long, short = 'n')]
        name: Option<String>,
        /// Maximum rows to print
        #[arg(long, default_value_t = 100)]
        limit: usize,
        /// Workspace to act on; defaults to BB_WORKSPACE, then the git remote
        #[arg(long)]
        workspace: Option<String>,
    },
}

#[derive(Subcommand)]
enum PrCommand {
    /// List pull requests
    #[command(alias = "l", alias = "ls")]
    List {
        /// Only show pull requests targeting this branch
        destination: Option<String>,
        /// State filter: OPEN, MERGED, DECLINED, SUPERSEDED, DRAFT or ALL
        #[arg(long, default_value = "OPEN")]
        state: String,
        /// Only pull requests this person is tagged to review
        #[arg(long)]
        reviewer: Option<String>,
        /// Only pull requests opened by this person; `@me` for yourself
        #[arg(long)]
        author: Option<String>,
        /// Your own review state on the pull request
        #[arg(long, value_enum)]
        review_state: Option<commands::pr_list::ReviewStateArg>,
        /// Only pull requests waiting on your review
        #[arg(long)]
        needs_my_review: bool,
        /// Show the build status column (one extra request per pull request)
        #[arg(long)]
        build: bool,
        /// Only pull requests whose build rolls up to this state
        #[arg(long, value_enum)]
        build_status: Option<commands::pr_list::BuildStateArg>,
    },
    /// Print the raw diff for a pull request
    #[command(alias = "d")]
    Diff { id: u64 },
    /// List files changed in a pull request
    Files { id: u64 },
    /// List commits in a pull request
    #[command(alias = "c")]
    Commits { id: u64 },
    /// Show the build statuses reported on a pull request
    Build { id: u64 },
    /// Point an open pull request at a different destination branch
    Retarget {
        id: u64,
        /// The branch the pull request should merge into
        #[arg(long = "to")]
        to: String,
    },
    /// Request changes on a pull request, after confirming
    #[command(name = "request-changes", alias = "rc")]
    RequestChanges {
        id: u64,
        /// Skip the confirmation prompt
        #[arg(long, short = 'y')]
        yes: bool,
    },
    /// Withdraw a change request, after confirming
    #[command(name = "no-request-changes", alias = "nrc")]
    NoRequestChanges {
        id: u64,
        /// Skip the confirmation prompt
        #[arg(long, short = 'y')]
        yes: bool,
    },
    /// Open a pull request
    Create {
        /// Target branch, or a comma-separated list of target branches
        target: String,
        /// Source branch (defaults to the current branch)
        source: Option<String>,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        description: Option<String>,
        /// Do not attach the repository's default reviewers
        #[arg(long)]
        no_default_reviewers: bool,
        /// Tag exactly these reviewers, comma-separated; a `{uuid}` is taken
        /// verbatim. Replaces the repository's default reviewers entirely
        #[arg(long, value_name = "NAMES")]
        reviewer: Option<String>,
        /// Prompt for title and description
        #[arg(long, short = 'i')]
        interactive: bool,
        /// Open the new pull request in a browser
        #[arg(long, short = 'w')]
        web: bool,
        /// Delete the source branch once merged
        #[arg(long)]
        close_source_branch: bool,
    },
    /// Show a pull request with its comments
    #[command(alias = "show", alias = "v")]
    View {
        id: u64,
        /// Hide inline threads that have been resolved
        #[arg(long)]
        unresolved: bool,
        /// Skip the pull request header and print only comments
        #[arg(long)]
        comments_only: bool,
    },
    /// Show, add or remove the reviewers tagged on a pull request
    #[command(args_conflicts_with_subcommands = true)]
    Reviewers {
        /// Pull request id (omit when using add/remove)
        id: Option<u64>,
        #[command(subcommand)]
        command: Option<ReviewersCommand>,
    },
    /// Comment on a pull request
    Comment {
        id: u64,
        /// Comment text
        #[arg(long, short = 'b')]
        body: Option<String>,
        /// Read the comment text from stdin
        #[arg(long)]
        body_stdin: bool,
        /// Attach the comment to this file
        #[arg(long, short = 'f')]
        file: Option<String>,
        /// Attach the comment to this line of --file
        #[arg(long, short = 'l')]
        line: Option<u64>,
        /// Reply to an existing comment id
        #[arg(long)]
        reply_to: Option<u64>,
        /// Open the comment in a browser
        #[arg(long, short = 'w')]
        web: bool,
    },
    /// Mark a comment thread as resolved, after confirming
    Resolve {
        id: u64,
        /// Id of the thread's first comment
        comment: u64,
        /// Approve without the confirmation prompt
        #[arg(long, short = 'y')]
        yes: bool,
    },
    /// Reopen a resolved comment thread
    Unresolve {
        id: u64,
        /// Id of the thread's first comment
        comment: u64,
    },
    /// List your pull requests across every repository you can see
    Mine {
        /// Which pull requests: author, reviewer or all
        #[arg(long, value_enum, default_value = "all")]
        role: commands::pr_mine::RoleArg,
        /// State filter: OPEN, MERGED, DECLINED, SUPERSEDED or ALL
        #[arg(long, default_value = "OPEN")]
        state: String,
        /// Workspace(s) to scan, comma-separated. Falls back to BB_WORKSPACE,
        /// then to the workspace of the current git checkout.
        #[arg(long)]
        workspace: Option<String>,
        /// Most recently updated repositories to scan per workspace
        #[arg(long, default_value_t = 30)]
        repo_limit: usize,
        /// Show the build status column (one extra request per pull request)
        #[arg(long)]
        build: bool,
    },
}

#[derive(Subcommand)]
enum ReviewersCommand {
    /// List the reviewers on a pull request and what each has decided
    #[command(alias = "l", alias = "ls")]
    List { id: u64 },
    /// Tag one or more reviewers, comma-separated
    Add {
        id: u64,
        /// Reviewer names, comma-separated; a `{uuid}` is taken verbatim
        names: String,
    },
    /// Untag one or more reviewers, comma-separated
    #[command(alias = "rm")]
    Remove {
        id: u64,
        /// Reviewer names, comma-separated; a `{uuid}` is taken verbatim
        names: String,
    },
}

#[derive(Subcommand)]
enum AuthCommand {
    /// Store an atlassian api token in the os keyring
    #[command(long_about = bb_cli::commands::auth::login_long_about())]
    Login {
        /// Atlassian account email
        #[arg(long)]
        email: Option<String>,
        /// Read the api token from stdin instead of prompting
        #[arg(long)]
        token_stdin: bool,
    },
    /// Show the active account with the token redacted
    Status,
    /// Remove stored credentials
    Logout,
}

/// `brew upgrade bb` and `cargo install` replace the binary without running any
/// of our code, so an installed skill file would otherwise keep describing an
/// older CLI until someone noticed `bb skill status` saying `stale` and re-ran
/// the install. Every tracked entry records the version that wrote it, so the
/// check is a string compare and costs nothing once everything is current.
///
/// Five properties this keeps, each with a test: it never overwrites a locally
/// edited file (`refresh_tracked` reports those as skipped), it never writes to
/// stdout so `--json` stays pure, it never fails the command the user actually
/// asked for, `BB_SKILL_NO_AUTO_REFRESH=1` turns it off, and `refresh_tracked`
/// stamps the running version onto every entry it looked at — including skipped
/// ones — so this fires once per upgrade rather than on every invocation.
fn auto_refresh_skills(format: Format) {
    if std::env::var_os("BB_SKILL_NO_AUTO_REFRESH").is_some() {
        return;
    }
    let (entries, _warning) = skill::load_state();
    if entries.is_empty() || !skill::tracked_version_differs(&entries) {
        return;
    }
    // `Preserve` because this call runs ahead of a command the user did not
    // ask to refresh anything with — a file they deliberately deleted must
    // stay deleted here. Only explicit `bb skill install`/`bb update` restore
    // a missing file.
    match skill::refresh_tracked(skill::MissingPolicy::Preserve) {
        Ok(outcomes) => {
            // Refreshed, pruned and failed are different events and the line
            // must not conflate them: "refreshed 2" when some were actually
            // dropped or left broken reads as a write that never happened.
            let refreshed = outcomes
                .iter()
                .filter(|o| o.action == skill::Action::Refreshed)
                .count();
            let pruned = outcomes
                .iter()
                .filter(|o| o.action == skill::Action::Pruned)
                .count();
            let failed = outcomes
                .iter()
                .filter(|o| o.action == skill::Action::Failed)
                .count();
            if (refreshed > 0 || pruned > 0 || failed > 0) && !format.is_json() {
                let mut parts = Vec::new();
                if refreshed > 0 {
                    parts.push(format!(
                        "refreshed {refreshed} skill file{}",
                        if refreshed == 1 { "" } else { "s" }
                    ));
                }
                if pruned > 0 {
                    parts.push(format!(
                        "forgot {pruned} skill path{} that no longer exist{}",
                        if pruned == 1 { "" } else { "s" },
                        if pruned == 1 { "s" } else { "" }
                    ));
                }
                // Named once, as a count — not per path — so a read-only
                // checkout does not spam a line per tracked entry on every
                // single invocation.
                if failed > 0 {
                    parts.push(format!(
                        "could not refresh {failed} skill file{}",
                        if failed == 1 { "" } else { "s" }
                    ));
                }
                output::warn(&format!(
                    "{} for bb {}",
                    parts.join(", "),
                    env!("CARGO_PKG_VERSION")
                ));
            }
        }
        // The user asked for something else. A read-only filesystem or a
        // vanished directory must not turn their command into a failure. Per
        // entry write failures no longer reach here at all (see
        // `refresh_tracked`'s `Action::Failed`) — only `save_state` itself
        // failing does, which is rare enough that warning every time is fine.
        Err(err) => {
            if !format.is_json() {
                output::warn(&format!("could not refresh agent skills: {err}"));
            }
        }
    }
}

async fn run(cli: Cli) -> Result<()> {
    let format = Format::from_json_flag(cli.json);
    auto_refresh_skills(format);
    // `bb update` asks the release api itself and reports the answer as its
    // whole output, so a passive notice ahead of it would be a duplicate
    // request and a duplicate line.
    if !matches!(cli.command, Command::Update) {
        update_check::maybe_notify(format, &commands::update::release_api_base()).await;
    }
    match cli.command {
        Command::Auth { command } => match command {
            AuthCommand::Login { email, token_stdin } => {
                commands::auth::login(email, token_stdin, format).await
            }
            AuthCommand::Status => commands::auth::status(format).await,
            AuthCommand::Logout => commands::auth::logout(format),
        },
        Command::Pr { command } => {
            if let PrCommand::Mine {
                role,
                state,
                workspace,
                repo_limit,
                build,
            } = command
            {
                return commands::pr_mine::run(
                    format,
                    commands::pr_mine::MineArgs {
                        role,
                        state,
                        workspace,
                        repo_limit,
                        build,
                    },
                )
                .await;
            }
            let ctx = commands::pr::Ctx::new(cli.repo.as_deref(), format)?;
            match command {
                PrCommand::List {
                    destination,
                    state,
                    reviewer,
                    author,
                    review_state,
                    needs_my_review,
                    build,
                    build_status,
                } => {
                    commands::pr_list::list(
                        &ctx,
                        commands::pr_list::ListArgs {
                            destination,
                            state,
                            reviewer,
                            author,
                            review_state,
                            needs_my_review,
                            build,
                            build_status,
                        },
                    )
                    .await
                }
                PrCommand::Diff { id } => commands::pr::diff(&ctx, id).await,
                PrCommand::Files { id } => commands::pr::files(&ctx, id).await,
                PrCommand::Commits { id } => commands::pr::commits(&ctx, id).await,
                PrCommand::Build { id } => commands::pr_build::run(&ctx, id).await,
                PrCommand::Retarget { id, to } => commands::pr_retarget::run(&ctx, id, &to).await,
                PrCommand::RequestChanges { id, yes } => {
                    commands::pr::request_changes(&ctx, id, yes).await
                }
                PrCommand::NoRequestChanges { id, yes } => {
                    commands::pr::unrequest_changes(&ctx, id, yes).await
                }
                PrCommand::Create {
                    target,
                    source,
                    title,
                    description,
                    no_default_reviewers,
                    reviewer,
                    interactive,
                    web,
                    close_source_branch,
                } => {
                    commands::pr::create(
                        &ctx,
                        commands::pr::CreateArgs {
                            target,
                            source,
                            title,
                            description,
                            no_default_reviewers,
                            reviewer,
                            interactive,
                            web,
                            close_source_branch,
                        },
                    )
                    .await
                }
                PrCommand::Reviewers { id, command } => match (id, command) {
                    (_, Some(ReviewersCommand::List { id })) => {
                        commands::pr_reviewers::list(&ctx, id).await
                    }
                    (_, Some(ReviewersCommand::Add { id, names })) => {
                        commands::pr_reviewers::add(&ctx, id, &names).await
                    }
                    (_, Some(ReviewersCommand::Remove { id, names })) => {
                        commands::pr_reviewers::remove(&ctx, id, &names).await
                    }
                    (Some(id), None) => commands::pr_reviewers::list(&ctx, id).await,
                    (None, None) => Err(bb_cli::error::BbError::Config(
                        "pass a pull request id, or `add`/`remove`".into(),
                    )),
                },
                PrCommand::View {
                    id,
                    unresolved,
                    comments_only,
                } => commands::pr_comments::view(&ctx, id, unresolved, comments_only).await,
                PrCommand::Comment {
                    id,
                    body,
                    body_stdin,
                    file,
                    line,
                    reply_to,
                    web,
                } => {
                    commands::pr_comments::comment(
                        &ctx,
                        commands::pr_comments::CommentArgs {
                            id,
                            body,
                            body_stdin,
                            file,
                            line,
                            reply_to,
                            web,
                        },
                    )
                    .await
                }
                PrCommand::Resolve { id, comment, yes } => {
                    commands::pr_comments::resolve(&ctx, id, comment, yes).await
                }
                PrCommand::Unresolve { id, comment } => {
                    commands::pr_comments::unresolve(&ctx, id, comment).await
                }
                PrCommand::Mine { .. } => {
                    Err(BbError::Config("pr mine does not take a repository".into()))
                }
            }
        }
        Command::Branch { command } => {
            let ctx = commands::pr::Ctx::new(cli.repo.as_deref(), format)?;
            match command {
                BranchCommand::List { user, name, limit } => {
                    commands::branch::list(&ctx, user, name, limit).await
                }
            }
        }
        Command::Project { command } => match command {
            ProjectCommand::List {
                name,
                limit,
                workspace,
            } => {
                let ctx = workspace::WorkspaceCtx::new(workspace.as_deref(), format)?;
                commands::project::list(&ctx, name, limit).await
            }
        },
        Command::Repo { command } => match command {
            RepoCommand::Create {
                name,
                project,
                description,
                public,
                workspace,
            } => {
                let ctx = workspace::WorkspaceCtx::new(workspace.as_deref(), format)?;
                commands::repo::create(&ctx, name, project, description, public).await
            }
            RepoCommand::List {
                project,
                name,
                limit,
                workspace,
            } => {
                let ctx = workspace::WorkspaceCtx::new(workspace.as_deref(), format)?;
                commands::repo::list(&ctx, project, name, limit).await
            }
        },
        Command::Browse {
            print,
            pr,
            branches,
        } => {
            let target = if let Some(id) = pr {
                Some(commands::browse::BrowseTarget::Pr(id))
            } else if branches {
                Some(commands::browse::BrowseTarget::Branches)
            } else {
                None
            };
            commands::browse::browse(cli.repo.as_deref(), target, print, format)
        }
        Command::Completions { shell } => {
            commands::completions::generate::<Cli>(shell);
            Ok(())
        }
        Command::Update => {
            commands::update::run(format, &commands::update::release_api_base()).await
        }
        Command::Skill { command } => match command {
            SkillCommand::Install {
                agent,
                global,
                force,
                skill,
                all,
            } => commands::skill::install(
                format,
                agent.as_deref(),
                global,
                force,
                skill.as_deref(),
                all,
            ),
            SkillCommand::Status => commands::skill::status(format),
            SkillCommand::Uninstall {
                global,
                force,
                skill,
            } => commands::skill::uninstall(format, global, force, skill.as_deref()),
        },
    }
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    // `clap`'s `conflicts_with` cannot span a global, top-level arg and an id
    // that only exists on one nested subcommand's own `Command` node, so this
    // is enforced by hand instead, using clap's own error rendering — `pr
    // mine` is not repository-scoped, and accepting `-R`/`--repo` there would
    // silently discard it (see `PrCommand::Mine { .. }`'s residual match arm).
    if cli.repo.is_some()
        && matches!(&cli.command, Command::Pr { command } if matches!(command, PrCommand::Mine { .. }))
    {
        let mut cmd = Cli::command();
        cmd.error(
            clap::error::ErrorKind::ArgumentConflict,
            "the argument '--repo' cannot be used with 'pr mine': it scans every repository, not one",
        )
        .exit();
    }
    if let Err(err) = run(cli).await {
        eprintln!("error: {err}");
        std::process::exit(err.exit_code());
    }
}
