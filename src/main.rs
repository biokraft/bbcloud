#![forbid(unsafe_code)]

use bb_cli::commands;
use bb_cli::error::Result;
use bb_cli::output::Format;
use clap::{Parser, Subcommand};

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
    },
    /// Print the raw diff for a pull request
    #[command(alias = "d")]
    Diff { id: u64 },
    /// List files changed in a pull request
    Files { id: u64 },
    /// List commits in a pull request
    #[command(alias = "c")]
    Commits { id: u64 },
    /// Request changes on a pull request
    #[command(name = "request-changes", alias = "rc")]
    RequestChanges { id: u64 },
    /// Withdraw a change request
    #[command(name = "no-request-changes", alias = "nrc")]
    NoRequestChanges { id: u64 },
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

async fn run(cli: Cli) -> Result<()> {
    let format = Format::from_json_flag(cli.json);
    match cli.command {
        Command::Auth { command } => match command {
            AuthCommand::Login { email, token_stdin } => {
                commands::auth::login(email, token_stdin, format).await
            }
            AuthCommand::Status => commands::auth::status(format).await,
            AuthCommand::Logout => commands::auth::logout(format),
        },
        Command::Pr { command } => {
            let ctx = commands::pr::Ctx::new(cli.repo.as_deref(), format)?;
            match command {
                PrCommand::List {
                    destination,
                    state,
                    reviewer,
                    author,
                    review_state,
                    needs_my_review,
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
                        },
                    )
                    .await
                }
                PrCommand::Diff { id } => commands::pr::diff(&ctx, id).await,
                PrCommand::Files { id } => commands::pr::files(&ctx, id).await,
                PrCommand::Commits { id } => commands::pr::commits(&ctx, id).await,
                PrCommand::RequestChanges { id } => commands::pr::request_changes(&ctx, id).await,
                PrCommand::NoRequestChanges { id } => {
                    commands::pr::unrequest_changes(&ctx, id).await
                }
                PrCommand::Create {
                    target,
                    source,
                    title,
                    description,
                    no_default_reviewers,
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
    }
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    if let Err(err) = run(cli).await {
        eprintln!("error: {err}");
        std::process::exit(err.exit_code());
    }
}
