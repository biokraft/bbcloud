use crate::api::models::{Commit, DiffStatEntry, Endpoint, PullRequest, User};
use crate::api::{repo_path, Client, Page};
use crate::commands::pr::Ctx;
use crate::error::{BbError, Result};
use crate::git;
use crate::output::{self, Format};
use crate::repo::RepoSlug;
use crate::users::UserPool;
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use futures::stream::{self, StreamExt};
use serde::Serialize;
use std::collections::{HashMap, HashSet};

const MAX_IN_FLIGHT: usize = 8;
const MAX_COMMITS_PER_PATH: usize = 100;
/// The ceiling on paths read in one report, so a machine-generated branch with
/// thousands of files cannot fan out into thousands of requests. It is separate
/// from `MAX_COMMITS_PER_PATH`, which bounds one path's history, and it is
/// reported rather than applied silently.
const MAX_PATHS: usize = 100;

/// Which side of the comparison an author's evidence came from. Target history
/// is who maintains the file; source-only history is who else worked on this
/// branch. They are not the same population and are never merged into one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Scope {
    /// Commits reachable from the target branch — the file's maintainers.
    Target,
    /// Commits on the source branch that the target does not already contain.
    Source,
}

#[derive(Debug, Serialize)]
struct PullRequestIdentity {
    id: u64,
    source: String,
    target: String,
    source_repository: String,
    target_repository: String,
    source_revision: Option<String>,
    target_revision: Option<String>,
}

#[derive(Debug, Serialize)]
struct ProspectiveIdentity {
    source: String,
    target: String,
    source_repository: String,
    target_repository: String,
    source_revision: Option<String>,
    target_revision: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SuggestArgs {
    pub pr: Option<u64>,
    pub target: Option<String>,
    pub source: Option<String>,
    pub since: String,
    pub limit: usize,
    pub file_limit: usize,
    pub acknowledge_private_data: bool,
}

/// What the target repository can prove about a suggested reviewer.
///
/// Three states, because a name that could not be proven taggable is not the
/// same claim as one proven untaggable, and a report that conflates them either
/// hides a blocker or invents one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum CanReview {
    /// Listed in this repository's own permission configuration.
    Yes,
    /// The pool is complete and the person is not in it.
    No,
    /// No complete pool was available, or the row is not a direct permission.
    Unknown,
}

#[derive(Debug, Serialize)]
struct Suggestion {
    name: String,
    uuid: String,
    commit_count: usize,
    target_commits: usize,
    source_commits: usize,
    files: Vec<String>,
    last_commit_on: Option<String>,
    can_review: CanReview,
}

#[derive(Debug, Serialize)]
struct HistoryError {
    path: String,
    message: String,
}

#[derive(Debug, Serialize)]
struct Report {
    #[serde(skip_serializing_if = "Option::is_none")]
    pull_request: Option<PullRequestIdentity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    prospective: Option<ProspectiveIdentity>,
    since: String,
    files_scanned: usize,
    files_skipped: usize,
    /// Distinct paths history was actually read for. Exceeds `files_scanned`
    /// when a rename contributed both its paths.
    paths_scanned: usize,
    /// Distinct paths dropped by the hard path ceiling. Non-zero means the
    /// change was larger than one report is allowed to read.
    paths_skipped: usize,
    history_complete: bool,
    errors: Vec<HistoryError>,
    suggestions: Vec<Suggestion>,
}

struct History {
    commits: Vec<Commit>,
    complete: bool,
}

/// The paths to read, and an honest account of what was left out.
///
/// `file_limit` caps diffstat *entries*; `MAX_PATHS` caps distinct *paths*,
/// because a rename is one entry and two paths. Both cuts are returned so the
/// report can state them: a suggestion list that silently dropped half the
/// change is the same failure as an empty one that exits zero.
struct PathPlan {
    /// The paths to read history for, in the order the diffstat listed them.
    paths: Vec<String>,
    files_scanned: usize,
    files_skipped: usize,
    paths_skipped: usize,
}

fn plan_paths(diffstat: &[DiffStatEntry], file_limit: usize) -> PathPlan {
    let scanned = &diffstat[..diffstat.len().min(file_limit)];
    let mut paths: Vec<String> = Vec::new();
    let mut seen = HashSet::new();
    for entry in scanned {
        for path in entry.paths() {
            if path != "-" && seen.insert(path.to_string()) {
                paths.push(path.to_string());
            }
        }
    }
    let paths_scanned = paths.len();
    paths.truncate(MAX_PATHS);
    PathPlan {
        paths,
        files_scanned: scanned.len(),
        files_skipped: diffstat.len() - scanned.len(),
        paths_skipped: paths_scanned - MAX_PATHS.min(paths_scanned),
    }
}

#[derive(Default)]
struct Candidate {
    user: User,
    target_commits: HashSet<String>,
    source_commits: HashSet<String>,
    files: HashSet<String>,
    last_commit_on: Option<DateTime<Utc>>,
}

fn parse_since(value: &str) -> Result<Duration> {
    let value = value.trim().to_ascii_lowercase();
    let split = value
        .find(|character: char| !character.is_ascii_digit())
        .unwrap_or(value.len());
    let (amount, suffix) = value.split_at(split);
    let amount = amount.parse::<u64>().map_err(|_| {
        BbError::Config(format!(
            "invalid --since value `{value}` — use values such as 30d, 12w, 6mo or 1y"
        ))
    })?;
    let days = match suffix {
        "" | "d" => 1,
        "w" => 7,
        "mo" | "m" | "month" | "months" => 30,
        "y" | "year" | "years" => 365,
        _ => {
            return Err(BbError::Config(format!(
                "invalid --since suffix `{suffix}` — use d, w, mo or y"
            )))
        }
    };
    if amount == 0 || amount > 3650 {
        return Err(BbError::Config(
            "--since must be between 1 and 3650 units".into(),
        ));
    }
    // Bounded by the check above and by `days`, which is at most 365, so the
    // product is at most 1_332_250 and cannot overflow either type. An overflow
    // arm here would be a branch that can never fire.
    Ok(Duration::days((amount * days) as i64))
}

/// The repository an endpoint really points at. A pull request from a fork
/// names its source repository, and falling back to the target's would read
/// history from the wrong project entirely.
fn endpoint_repo(endpoint: Option<&Endpoint>, fallback: &RepoSlug) -> Result<RepoSlug> {
    let name = endpoint
        .and_then(|endpoint| endpoint.repository.as_ref())
        .and_then(|repository| repository.full_name.as_deref());
    match name {
        Some(name) => RepoSlug::parse(name).map_err(|error| {
            BbError::Config(format!(
                "cannot read the source repository from bitbucket: {error}"
            ))
        }),
        None => Ok(fallback.clone()),
    }
}

fn endpoint_revision(endpoint: Option<&Endpoint>) -> Option<String> {
    endpoint
        .and_then(|endpoint| endpoint.commit.as_ref())
        .and_then(|commit| commit.hash.clone())
}

/// Resolve a branch to the commit it points at, so the report names a revision
/// instead of a name that can move between the request and the reading of it.
async fn resolve_revision(
    client: &Client,
    repository: &RepoSlug,
    revision: &str,
) -> Result<String> {
    let page: Page<Commit> = client
        .get_json(&repo_path(
            repository,
            &format!("/commits/{}?pagelen=1", urlencoding::encode(revision)),
        ))
        .await?;
    page.values
        .into_iter()
        .next()
        .and_then(|commit| commit.hash)
        .ok_or_else(|| {
            BbError::Config(format!(
                "branch `{revision}` has no commits to read history from"
            ))
        })
}

async fn history(
    client: &Client,
    repository: &RepoSlug,
    revision: &str,
    exclude: Option<&str>,
    path: &str,
) -> Result<History> {
    let mut suffix = format!(
        "/commits/{}?pagelen={MAX_COMMITS_PER_PATH}&path={}",
        urlencoding::encode(revision),
        urlencoding::encode(path)
    );
    if let Some(exclude) = exclude {
        suffix.push_str("&exclude=");
        suffix.push_str(&urlencoding::encode(exclude));
    }
    let page: Page<Commit> = client.get_json(&repo_path(repository, &suffix)).await?;
    Ok(History {
        complete: page.next.is_none() && page.values.len() < MAX_COMMITS_PER_PATH,
        commits: page.values,
    })
}

async fn history_for_path(
    client: &Client,
    repository: RepoSlug,
    revision: String,
    exclude: Option<String>,
    path: String,
    scope: Scope,
) -> (String, Scope, Result<History>) {
    let result = history(client, &repository, &revision, exclude.as_deref(), &path).await;
    (path, scope, result)
}

fn commit_key(commit: &Commit, path: &str) -> String {
    commit.hash.clone().unwrap_or_else(|| {
        format!(
            "{}|{}|{}",
            commit.date.as_deref().unwrap_or(""),
            commit
                .summary
                .as_ref()
                .and_then(|summary| summary.raw.as_deref())
                .unwrap_or(""),
            path
        )
    })
}

/// Records one commit's evidence for one author.
///
/// The uuid is passed separately because the caller has already established
/// that there is one: taking it from `user` again would leave a second path
/// through this function for an author with no uuid, which is the case the
/// caller exists to filter out.
fn add_candidate(
    candidates: &mut HashMap<String, Candidate>,
    uuid: &str,
    user: &User,
    commit: &Commit,
    path: &str,
    scope: Scope,
    commit_date: DateTime<Utc>,
) {
    let candidate = candidates
        .entry(uuid.to_string())
        .or_insert_with(|| Candidate {
            user: user.clone(),
            ..Candidate::default()
        });
    if candidate.user.display_name.is_none() {
        candidate.user.display_name = user.display_name.clone();
    }
    if candidate.user.nickname.is_none() {
        candidate.user.nickname = user.nickname.clone();
    }
    match scope {
        Scope::Target => {
            candidate.target_commits.insert(commit_key(commit, path));
        }
        Scope::Source => {
            candidate.source_commits.insert(commit_key(commit, path));
        }
    }
    candidate.files.insert(path.to_string());
    if candidate
        .last_commit_on
        .is_none_or(|last| commit_date > last)
    {
        candidate.last_commit_on = Some(commit_date);
    }
}

/// A 404 on one path means the file has no history there; a 403, a rate limit,
/// a 5xx or a transport failure says something about the whole request and
/// cannot be charged to a single file.
fn is_path_specific(error: &BbError) -> bool {
    matches!(error, BbError::NotFound | BbError::Api { status: 403, .. })
}

struct Inputs {
    identity_source: Option<PullRequestIdentity>,
    identity_prospective: Option<ProspectiveIdentity>,
    diffstat: Vec<DiffStatEntry>,
    source_repo: RepoSlug,
    target_repo: RepoSlug,
    source_revision: String,
    target_revision: String,
    /// Target history is read on the target repository. Source history is only
    /// subtracted from the target when both live in the same repository, where
    /// the target revision is actually reachable from the source branch.
    source_exclude: Option<String>,
    excluded: HashSet<String>,
}

pub async fn run(ctx: &Ctx, args: SuggestArgs) -> Result<()> {
    if !args.acknowledge_private_data {
        return Err(BbError::Config(
            "this reads commit history — private file paths, colleague names, dates and \
             account ids — pass --acknowledge-private-data, after telling the user what leaves \
             their machine"
                .into(),
        ));
    }
    if args.pr.is_some() == args.target.is_some() {
        return Err(BbError::Config(
            "pass either --pr ID or a target branch, not both and not neither".into(),
        ));
    }
    if args.pr.is_some() && args.source.is_some() {
        return Err(BbError::Config(
            "a source branch is only valid with a prospective target, not alongside --pr".into(),
        ));
    }
    let window = parse_since(&args.since)?;
    let cutoff = Utc::now() - window;

    let inputs = if let Some(id) = args.pr {
        pr_inputs(ctx, id).await?
    } else {
        prospective_inputs(ctx, &args).await?
    };
    let Inputs {
        identity_source,
        identity_prospective,
        diffstat,
        source_repo,
        target_repo,
        source_revision,
        target_revision,
        source_exclude,
        excluded,
    } = inputs;

    // `--file-limit` bounds how many *diffstat entries* are looked at, so the
    // flag the user set is the work actually done. A rename contributes two
    // paths and a removal one, so paths are counted after that cut and can
    // exceed it — reported separately rather than conflated.
    let PathPlan {
        paths,
        files_scanned,
        files_skipped,
        paths_skipped,
    } = plan_paths(&diffstat, args.file_limit);

    // The evidence is only exhaustive when nothing was cut and every path read
    // its whole history.
    let mut history_complete = files_skipped == 0 && paths_skipped == 0;
    let mut errors = Vec::new();
    let mut candidates: HashMap<String, Candidate> = HashMap::new();
    let mut succeeded = 0usize;
    let mut attempted = 0usize;

    let mut requests = Vec::new();
    for path in &paths {
        requests.push(history_for_path(
            &ctx.client,
            target_repo.clone(),
            target_revision.clone(),
            None,
            path.clone(),
            Scope::Target,
        ));
        // A fork's source branch does not contain the target's commits, so there
        // is nothing to subtract there; subtracting anyway would silently read
        // the wrong population. The branch is still read — just whole.
        requests.push(history_for_path(
            &ctx.client,
            source_repo.clone(),
            source_revision.clone(),
            source_exclude.clone(),
            path.clone(),
            Scope::Source,
        ));
    }

    let mut stream = stream::iter(requests).buffer_unordered(MAX_IN_FLIGHT);
    while let Some((path, scope, result)) = stream.next().await {
        attempted += 1;
        match result {
            Ok(history) => {
                succeeded += 1;
                if !history.complete {
                    history_complete = false;
                }
                for commit in history.commits {
                    let Some(author) = commit
                        .author
                        .as_ref()
                        .and_then(|author| author.user.as_ref())
                    else {
                        continue;
                    };
                    let Some(uuid) = author.uuid.as_deref() else {
                        continue;
                    };
                    if excluded.contains(uuid) {
                        continue;
                    }
                    let Some(date) = commit
                        .date
                        .as_deref()
                        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
                        .map(|date| date.with_timezone(&Utc))
                    else {
                        continue;
                    };
                    if date < cutoff {
                        continue;
                    }
                    add_candidate(&mut candidates, uuid, author, &commit, &path, scope, date);
                }
            }
            Err(error) if is_path_specific(&error) => {
                history_complete = false;
                errors.push(HistoryError {
                    path,
                    message: error.to_string(),
                });
            }
            // Anything else — auth, a rate limit, a server or network failure —
            // applies to the whole call, not to one file. Returning an empty
            // report as a success is how a retired endpoint hides for weeks.
            Err(error) => return Err(error),
        }
    }

    if attempted > 0 && succeeded == 0 {
        let detail = errors
            .first()
            .map(|error| format!(": {} ({})", error.message, error.path))
            .unwrap_or_default();
        return Err(BbError::Api {
            status: 404,
            message: format!("no file history could be read for any changed path{detail}"),
        });
    }

    errors.sort_by(|left, right| left.path.cmp(&right.path));

    let access = AccessPool::load(&ctx.client, &target_repo).await;

    let mut suggestions: Vec<Suggestion> = candidates
        .into_values()
        .map(|candidate| {
            let mut files: Vec<String> = candidate.files.into_iter().collect();
            files.sort();
            let uuid = candidate.user.uuid.clone().unwrap_or_default();
            Suggestion {
                name: candidate.user.name().to_string(),
                can_review: access.can_review(&uuid),
                uuid,
                commit_count: candidate.target_commits.len() + candidate.source_commits.len(),
                target_commits: candidate.target_commits.len(),
                source_commits: candidate.source_commits.len(),
                files,
                last_commit_on: candidate
                    .last_commit_on
                    .map(|date| date.to_rfc3339_opts(SecondsFormat::Secs, true)),
            }
        })
        .collect();
    // Maintainers of the file first; a coauthor of this branch is a weaker
    // signal than the person who owns the code, and the report must not blur
    // the two into one undifferentiated count.
    suggestions.sort_by(|left, right| {
        right
            .target_commits
            .cmp(&left.target_commits)
            .then_with(|| right.commit_count.cmp(&left.commit_count))
            .then_with(|| right.last_commit_on.cmp(&left.last_commit_on))
            .then_with(|| left.name.cmp(&right.name))
            .then_with(|| left.uuid.cmp(&right.uuid))
    });
    suggestions.truncate(args.limit);

    let report = Report {
        pull_request: identity_source,
        prospective: identity_prospective,
        since: cutoff.to_rfc3339_opts(SecondsFormat::Secs, true),
        files_scanned,
        files_skipped,
        paths_scanned: paths.len(),
        paths_skipped,
        history_complete,
        errors,
        suggestions,
    };
    match ctx.format {
        Format::Json => output::print_json(&report)?,
        Format::Human => {
            output::heading("reviewer suggestions");
            if let Some(pr) = &report.pull_request {
                output::info(&format!("#{} · {} → {}", pr.id, pr.source, pr.target));
                if pr.source_repository != pr.target_repository {
                    output::info(&format!(
                        "source {} · target {}",
                        pr.source_repository, pr.target_repository
                    ));
                }
            } else if let Some(prospective) = &report.prospective {
                output::info(&format!("{} → {}", prospective.source, prospective.target));
            }
            output::info(&format!(
                "since {} · files {} scanned · {} skipped · paths {} read · {} skipped",
                report.since,
                report.files_scanned,
                report.files_skipped,
                report.paths_scanned,
                report.paths_skipped
            ));
            if !report.history_complete {
                output::warn("history is partial; the evidence is not exhaustive");
            }
            if report.paths_skipped > 0 {
                output::warn(&format!(
                    "{} changed path(s) exceeded the {MAX_PATHS}-path ceiling for one report; \
                     their maintainers are not in this list",
                    report.paths_skipped
                ));
            }
            for error in &report.errors {
                output::warn(&format!("{}: {}", error.path, error.message));
            }
            if report.suggestions.is_empty() {
                output::info("no suggestions");
            } else {
                output::print_table(
                    &[
                        "NAME",
                        "COMMITS",
                        "IN TARGET",
                        "FILES",
                        "LAST COMMIT",
                        "CAN REVIEW",
                    ],
                    report
                        .suggestions
                        .iter()
                        .map(|suggestion| {
                            vec![
                                suggestion.name.clone(),
                                suggestion.commit_count.to_string(),
                                suggestion.target_commits.to_string(),
                                suggestion.files.join(", "),
                                suggestion
                                    .last_commit_on
                                    .clone()
                                    .unwrap_or_else(|| "-".into()),
                                match suggestion.can_review {
                                    CanReview::Yes => "yes".to_string(),
                                    CanReview::No => "no".to_string(),
                                    CanReview::Unknown => "unknown".to_string(),
                                },
                            ]
                        })
                        .collect(),
                );
            }
        }
    }
    Ok(())
}

async fn pr_inputs(ctx: &Ctx, id: u64) -> Result<Inputs> {
    let pr: PullRequest = ctx
        .client
        .get_json(&ctx.path(&format!("/pullrequests/{id}")))
        .await?;
    let source_branch = pr.source_branch().to_string();
    let target_branch = pr.destination_branch().to_string();
    let source_repo = endpoint_repo(pr.source.as_ref(), &ctx.slug)?;
    let target_repo = endpoint_repo(pr.destination.as_ref(), &ctx.slug)?;
    let source_revision =
        endpoint_revision(pr.source.as_ref()).unwrap_or_else(|| source_branch.clone());
    let target_revision =
        endpoint_revision(pr.destination.as_ref()).unwrap_or_else(|| target_branch.clone());
    let diffstat: Vec<DiffStatEntry> = ctx
        .client
        .paginate(&ctx.path(&format!("/pullrequests/{id}/diffstat?pagelen=100")))
        .await?;
    let mut excluded = HashSet::new();
    if let Some(uuid) = pr.author.as_ref().and_then(|author| author.uuid.as_deref()) {
        excluded.insert(uuid.to_string());
    }
    excluded.extend(
        pr.reviewers
            .iter()
            .filter_map(|reviewer| reviewer.uuid.clone()),
    );
    let same_repository = source_repo == target_repo;
    Ok(Inputs {
        identity_source: Some(PullRequestIdentity {
            id,
            source: source_branch.clone(),
            target: target_branch.clone(),
            source_repository: source_repo.to_string(),
            target_repository: target_repo.to_string(),
            source_revision: Some(source_revision.clone()),
            target_revision: Some(target_revision.clone()),
        }),
        identity_prospective: None,
        diffstat,
        source_repo,
        target_repo,
        source_revision,
        target_revision,
        // Subtracting the target only makes sense where the target's commits
        // are reachable from the source branch. In a fork they are not, and
        // asking to subtract them would read the wrong population.
        source_exclude: same_repository.then(|| target_branch.clone()),
        excluded,
    })
}

async fn prospective_inputs(ctx: &Ctx, args: &SuggestArgs) -> Result<Inputs> {
    let target = args.target.clone().unwrap_or_default();
    if target.trim().is_empty() {
        return Err(BbError::Config("a target branch is required".into()));
    }
    let source = match args.source.clone() {
        Some(source) if !source.trim().is_empty() => source,
        Some(_) => return Err(BbError::Config("source branch cannot be empty".into())),
        None => git::current_branch()?,
    };
    if source == target {
        return Err(BbError::Config(format!(
            "source and target are both `{source}`"
        )));
    }
    let spec = format!("{source}..{target}");
    let diffstat: Vec<DiffStatEntry> = ctx
        .client
        .paginate(&repo_path(
            &ctx.slug,
            &format!("/diffstat/{}?pagelen=100", urlencoding::encode(&spec)),
        ))
        .await?;
    // A prospective pull request has no endpoints to carry revisions, so both
    // are resolved once, here, and named in the report.
    let source_revision = resolve_revision(&ctx.client, &ctx.slug, &source).await?;
    let target_revision = resolve_revision(&ctx.client, &ctx.slug, &target).await?;
    // The person about to open this is its author, and bitbucket rejects the
    // author as a reviewer with a 400. Suggesting them wastes a round trip.
    let excluded = match ctx.client.get_json::<User>("/user").await {
        Ok(user) => user.uuid.into_iter().collect(),
        Err(BbError::NotFound) | Err(BbError::Api { status: 403, .. }) => HashSet::new(),
        Err(error) => return Err(error),
    };
    Ok(Inputs {
        identity_source: None,
        identity_prospective: Some(ProspectiveIdentity {
            source: source.clone(),
            target: target.clone(),
            source_repository: ctx.slug.to_string(),
            target_repository: ctx.slug.to_string(),
            source_revision: Some(source_revision.clone()),
            target_revision: Some(target_revision.clone()),
        }),
        diffstat,
        source_repo: ctx.slug.clone(),
        target_repo: ctx.slug.clone(),
        source_revision,
        target_revision,
        source_exclude: Some(target),
        excluded,
    })
}

/// What the target repository's own permissions can prove about a person.
struct AccessPool {
    /// Directly listed in the repository's permission configuration.
    explicit: HashSet<String>,
    /// Present in the pool at all, by any source.
    known: HashSet<String>,
    /// Whether every pool was read in full. Without this, absence proves
    /// nothing and `can_review` must stay `Unknown`.
    complete: bool,
}

impl AccessPool {
    /// Ownership of a file says nothing about access to the repository, so the
    /// two are reported separately rather than one implying the other.
    async fn load(client: &Client, repository: &RepoSlug) -> Self {
        match UserPool::load(client, repository).await {
            Ok(pool) => Self::from_pool(&pool),
            Err(_) => Self {
                explicit: HashSet::new(),
                known: HashSet::new(),
                complete: false,
            },
        }
    }

    fn from_pool(pool: &UserPool) -> Self {
        let mut explicit = HashSet::new();
        let mut known = HashSet::new();
        for entry in &pool.entries {
            if let Some(uuid) = entry.user.uuid.as_deref() {
                known.insert(uuid.to_string());
                if entry.eligibility == crate::users::RepositoryAccess::Explicit {
                    explicit.insert(uuid.to_string());
                }
            }
        }
        Self {
            explicit,
            known,
            complete: pool.incomplete.is_empty(),
        }
    }

    fn can_review(&self, uuid: &str) -> CanReview {
        if self.explicit.contains(uuid) {
            CanReview::Yes
        } else if self.complete && !self.known.contains(uuid) {
            CanReview::No
        } else {
            CanReview::Unknown
        }
    }
}

/// The window and access rules are pure decisions, so they are pinned here
/// rather than through five mocked HTTP round trips each. An accepted spelling
/// that stops being accepted is a silent behaviour change; a rejected one that
/// starts being accepted quietly widens a history read.
#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::users::RepositoryAccess;

    #[test]
    fn since_accepts_the_documented_spellings() {
        for (value, days) in [
            ("30d", 30),
            // A bare number carries no unit, so it is read as days.
            ("30", 30),
            ("12w", 84),
            ("6mo", 180),
            ("6m", 180),
            ("1y", 365),
            ("2years", 730),
            (" 45D ", 45),
        ] {
            assert_eq!(
                parse_since(value).unwrap(),
                Duration::days(days),
                "unexpected window for `{value}`"
            );
        }
    }

    #[test]
    fn since_rejects_what_it_cannot_bound() {
        for value in [
            "",
            "d",
            "abc",
            "30x",
            "0d",
            "3651d",
            "18446744073709551615y",
        ] {
            assert!(
                parse_since(value).is_err(),
                "`{value}` should not parse into a window"
            );
        }
    }

    #[test]
    fn access_separates_proof_of_access_from_a_missing_answer() {
        let complete = AccessPool {
            explicit: HashSet::from(["{yes}".to_string()]),
            known: HashSet::from(["{maybe}".to_string()]),
            complete: true,
        };
        assert_eq!(complete.can_review("{yes}"), CanReview::Yes);
        // Read in full, and not in it: that is a real no.
        assert_eq!(complete.can_review("{absent}"), CanReview::No);
        // In the pool, but only as a workspace member: no proof of access.
        assert_eq!(complete.can_review("{maybe}"), CanReview::Unknown);

        let partial = AccessPool {
            explicit: HashSet::new(),
            known: HashSet::new(),
            complete: false,
        };
        // An unreadable pool cannot rule anyone out.
        assert_eq!(partial.can_review("{absent}"), CanReview::Unknown);
    }

    #[test]
    fn an_access_pool_ignores_entries_without_a_uuid() {
        let pool = UserPool {
            entries: vec![crate::users::PoolEntry {
                user: User {
                    uuid: None,
                    account_id: Some("acct-1".into()),
                    display_name: Some("Nameless".into()),
                    nickname: None,
                },
                sources: vec!["workspace".into()],
                eligibility: RepositoryAccess::Explicit,
            }],
            incomplete: vec![],
        };
        let access = AccessPool::from_pool(&pool);
        assert!(access.explicit.is_empty());
        assert!(access.known.is_empty());
        // A complete pool with nobody in it means nobody can be tagged.
        assert!(access.complete);
    }

    /// clap rejects `--pr` alongside a positional target, but `run` is public
    /// and a library caller can hand it both. The guard has to hold on its own.
    #[tokio::test]
    async fn run_refuses_an_ambiguous_or_empty_subject() {
        let server = wiremock::MockServer::start().await;
        let ctx = Ctx {
            client: Client::new(
                crate::credentials::Credentials {
                    email: "dev@example.com".into(),
                    token: crate::secret::SecretString::from("t0ken-value"),
                },
                server.uri(),
            )
            .unwrap(),
            slug: crate::repo::RepoSlug::parse("acme/widgets").unwrap(),
            format: Format::Json,
        };
        let base = SuggestArgs {
            pr: None,
            target: None,
            source: None,
            since: "12mo".into(),
            limit: 5,
            file_limit: 25,
            acknowledge_private_data: true,
        };
        // Neither a pull request nor a target.
        let err = run(&ctx, base.clone()).await.unwrap_err();
        assert!(err.to_string().contains("either --pr"), "{err}");

        // Both a pull request and a target.
        let err = run(
            &ctx,
            SuggestArgs {
                pr: Some(7),
                target: Some("main".into()),
                ..base.clone()
            },
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("either --pr"), "{err}");

        // A prospective target has no use for an explicit source alongside a pr.
        let err = run(
            &ctx,
            SuggestArgs {
                pr: Some(7),
                source: Some("feature/x".into()),
                ..base
            },
        )
        .await
        .unwrap_err();
        assert!(
            err.to_string().contains("source branch is only valid"),
            "{err}"
        );

        assert!(
            server
                .received_requests()
                .await
                .unwrap_or_default()
                .is_empty(),
            "a rejected subject must not have touched the api"
        );
    }

    /// Bitbucket returns a hash for every commit, so `commit_key` only falls
    /// back when a response is malformed. The fallback is deliberately
    /// approximate and errs toward over-counting: two hashless commits sharing
    /// a date and a summary ("fix typo" twice in one day) must not collapse
    /// into one, because that would silently under-count a real contributor.
    /// The cost is that one hashless commit spanning N files counts N times.
    #[test]
    fn a_hashless_commit_is_counted_once_per_path_not_merged() {
        let commit = Commit {
            hash: None,
            summary: None,
            author: None,
            date: Some("2026-08-20T10:00:00+00:00".into()),
        };
        // Stable, and never empty: the same (commit, path) is the same evidence.
        let key = commit_key(&commit, "src/lib.rs");
        assert!(!key.is_empty());
        assert_eq!(key, commit_key(&commit, "src/lib.rs"));
        // Two files of the same commit stay distinguishable rather than merging.
        assert_ne!(key, commit_key(&commit, "src/main.rs"));

        // A real hash wins outright and is path-independent: one commit
        // touching many files is still one commit.
        let hashed = Commit {
            hash: Some("abc123".into()),
            summary: None,
            author: None,
            date: Some("2026-08-20T10:00:00+00:00".into()),
        };
        assert_eq!(commit_key(&hashed, "src/lib.rs"), "abc123");
        assert_eq!(commit_key(&hashed, "src/main.rs"), "abc123");
    }
}
