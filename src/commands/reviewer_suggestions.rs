use crate::api::models::{Commit, DiffStatEntry, Endpoint, PullRequest, User};
use crate::api::{repo_path, Client, Page};
use crate::commands::pr::Ctx;
use crate::error::{BbError, Result};
use crate::git;
use crate::output::{self, Format};
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use futures::stream::{self, StreamExt};
use serde::Serialize;
use std::collections::{HashMap, HashSet};

const MAX_IN_FLIGHT: usize = 8;
const MAX_COMMITS_PER_PATH: usize = 100;

#[derive(Debug)]
pub struct SuggestArgs {
    pub pr: Option<u64>,
    pub target: Option<String>,
    pub source: Option<String>,
    pub since: String,
    pub limit: usize,
    pub file_limit: usize,
}

#[derive(Debug, Serialize)]
struct PullRequestIdentity {
    id: u64,
    source: String,
    target: String,
}

#[derive(Debug, Serialize)]
struct ProspectiveIdentity {
    source: String,
    target: String,
}

#[derive(Debug, Serialize)]
struct Suggestion {
    name: String,
    uuid: String,
    commit_count: usize,
    files: Vec<String>,
    last_commit_on: Option<String>,
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
    history_complete: bool,
    errors: Vec<HistoryError>,
    suggestions: Vec<Suggestion>,
}

struct History {
    commits: Vec<Commit>,
    complete: bool,
}

struct Candidate {
    user: User,
    commits: HashSet<String>,
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
    let total_days = amount
        .checked_mul(days)
        .ok_or_else(|| BbError::Config("--since is too large".into()))?;
    let total_days =
        i64::try_from(total_days).map_err(|_| BbError::Config("--since is too large".into()))?;
    Ok(Duration::days(total_days))
}

fn endpoint_repo(
    endpoint: Option<&Endpoint>,
    fallback: &crate::repo::RepoSlug,
) -> crate::repo::RepoSlug {
    endpoint
        .and_then(|endpoint| endpoint.repository.as_ref())
        .and_then(|repository| repository.full_name.as_deref())
        .and_then(|name| crate::repo::RepoSlug::parse(name).ok())
        .unwrap_or_else(|| fallback.clone())
}

async fn history(
    client: &Client,
    repository: &crate::repo::RepoSlug,
    source_branch: &str,
    exclude_target: Option<&str>,
    path: &str,
) -> Result<History> {
    let mut suffix = format!(
        "/commits/{}?pagelen={MAX_COMMITS_PER_PATH}&path={}",
        urlencoding::encode(source_branch),
        urlencoding::encode(path)
    );
    if let Some(target) = exclude_target {
        suffix.push_str("&exclude=");
        suffix.push_str(&urlencoding::encode(target));
    }
    let page: Page<Commit> = client.get_json(&repo_path(repository, &suffix)).await?;
    Ok(History {
        complete: page.next.is_none() && page.values.len() < MAX_COMMITS_PER_PATH,
        commits: page.values,
    })
}

async fn history_for_path(
    client: &Client,
    repository: crate::repo::RepoSlug,
    source_branch: String,
    exclude_target: Option<String>,
    path: String,
) -> (String, Result<History>) {
    let result = history(
        client,
        &repository,
        &source_branch,
        exclude_target.as_deref(),
        &path,
    )
    .await;
    (path, result)
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

fn add_candidate(
    candidates: &mut HashMap<String, Candidate>,
    user: &User,
    commit: &Commit,
    path: &str,
    commit_date: DateTime<Utc>,
) {
    let Some(uuid) = user.uuid.as_deref() else {
        return;
    };
    let candidate = candidates
        .entry(uuid.to_string())
        .or_insert_with(|| Candidate {
            user: user.clone(),
            commits: HashSet::new(),
            files: HashSet::new(),
            last_commit_on: None,
        });
    if candidate.user.display_name.is_none() {
        candidate.user.display_name = user.display_name.clone();
    }
    if candidate.user.nickname.is_none() {
        candidate.user.nickname = user.nickname.clone();
    }
    candidate.commits.insert(commit_key(commit, path));
    candidate.files.insert(path.to_string());
    if candidate
        .last_commit_on
        .is_none_or(|last| commit_date > last)
    {
        candidate.last_commit_on = Some(commit_date);
    }
}

pub async fn run(ctx: &Ctx, args: SuggestArgs) -> Result<()> {
    if args.pr.is_some() == args.target.is_some() {
        return Err(BbError::Config(
            "pass either --pr ID or a target branch, not both and not neither".into(),
        ));
    }
    if args.pr.is_some() && args.source.is_some() {
        return Err(BbError::Config(
            "--source is only valid with a prospective target".into(),
        ));
    }
    let window = parse_since(&args.since)?;
    let cutoff = Utc::now() - window;

    let (pull_request, prospective, diffstat, source_repo, source_branch, exclude_target, excluded) =
        if let Some(id) = args.pr {
            let pr: PullRequest = ctx
                .client
                .get_json(&ctx.path(&format!("/pullrequests/{id}")))
                .await?;
            let source_branch = pr.source_branch().to_string();
            let target_branch = pr.destination_branch().to_string();
            let source_repo = endpoint_repo(pr.source.as_ref(), &ctx.slug);
            let target_repo = endpoint_repo(pr.destination.as_ref(), &ctx.slug);
            let diffstat: Vec<DiffStatEntry> = ctx
                .client
                .get_json::<Page<DiffStatEntry>>(
                    &ctx.path(&format!("/pullrequests/{id}/diffstat?pagelen=100")),
                )
                .await?
                .values;
            let mut excluded = HashSet::new();
            if let Some(uuid) = pr.author.as_ref().and_then(|author| author.uuid.as_deref()) {
                excluded.insert(uuid.to_string());
            }
            excluded.extend(
                pr.reviewers
                    .iter()
                    .filter_map(|reviewer| reviewer.uuid.clone()),
            );
            (
                Some(PullRequestIdentity {
                    id,
                    source: source_branch.clone(),
                    target: target_branch.clone(),
                }),
                None,
                diffstat,
                source_repo.clone(),
                source_branch,
                (source_repo == target_repo).then_some(target_branch),
                excluded,
            )
        } else {
            let target = args.target.unwrap_or_default();
            if target.trim().is_empty() {
                return Err(BbError::Config("a target branch is required".into()));
            }
            let source = match args.source {
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
            let path = repo_path(
                &ctx.slug,
                &format!("/diffstat/{}?pagelen=100", urlencoding::encode(&spec)),
            );
            let diffstat: Vec<DiffStatEntry> = ctx
                .client
                .get_json::<Page<DiffStatEntry>>(&path)
                .await?
                .values;
            (
                None,
                Some(ProspectiveIdentity {
                    source: source.clone(),
                    target: target.clone(),
                }),
                diffstat,
                ctx.slug.clone(),
                source,
                Some(target),
                HashSet::new(),
            )
        };

    let all_paths: Vec<String> = diffstat
        .iter()
        .map(|entry| entry.path().to_string())
        .filter(|path| path != "-")
        .collect();
    let files_scanned = all_paths.len().min(args.file_limit);
    let files_skipped = all_paths.len().saturating_sub(files_scanned)
        + (diffstat.len().saturating_sub(all_paths.len()));
    let paths: Vec<String> = all_paths.into_iter().take(files_scanned).collect();

    let mut history_complete = files_skipped == 0;
    let mut errors = Vec::new();
    let mut candidates: HashMap<String, Candidate> = HashMap::new();
    let requests = paths.into_iter().map(|path| {
        history_for_path(
            &ctx.client,
            source_repo.clone(),
            source_branch.clone(),
            exclude_target.clone(),
            path,
        )
    });
    let results = stream::iter(requests)
        .buffer_unordered(MAX_IN_FLIGHT)
        .collect::<Vec<_>>()
        .await;
    for (path, result) in results {
        match result {
            Ok(history) => {
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
                    add_candidate(&mut candidates, author, &commit, &path, date);
                }
            }
            Err(BbError::Auth) => return Err(BbError::Auth),
            Err(error) => {
                history_complete = false;
                errors.push(HistoryError {
                    path,
                    message: error.to_string(),
                });
            }
        }
    }

    errors.sort_by(|left, right| left.path.cmp(&right.path));

    let mut suggestions: Vec<Suggestion> = candidates
        .into_values()
        .map(|candidate| {
            let mut files: Vec<String> = candidate.files.into_iter().collect();
            files.sort();
            Suggestion {
                name: candidate.user.name().to_string(),
                uuid: candidate.user.uuid.unwrap_or_default(),
                commit_count: candidate.commits.len(),
                files,
                last_commit_on: candidate
                    .last_commit_on
                    .map(|date| date.to_rfc3339_opts(SecondsFormat::Secs, true)),
            }
        })
        .collect();
    suggestions.sort_by(|left, right| {
        right
            .commit_count
            .cmp(&left.commit_count)
            .then_with(|| right.last_commit_on.cmp(&left.last_commit_on))
            .then_with(|| left.name.cmp(&right.name))
            .then_with(|| left.uuid.cmp(&right.uuid))
    });
    suggestions.truncate(args.limit);

    let report = Report {
        pull_request,
        prospective,
        since: cutoff.to_rfc3339_opts(SecondsFormat::Secs, true),
        files_scanned,
        files_skipped,
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
            } else if let Some(prospective) = &report.prospective {
                output::info(&format!("{} → {}", prospective.source, prospective.target));
            }
            output::info(&format!(
                "since {} · files {} scanned · {} skipped",
                report.since, report.files_scanned, report.files_skipped
            ));
            if !report.history_complete {
                output::warn("history is partial; the evidence is not exhaustive");
            }
            for error in &report.errors {
                output::warn(&format!("{}: {}", error.path, error.message));
            }
            if report.suggestions.is_empty() {
                output::info("no suggestions");
            } else {
                output::print_table(
                    &["NAME", "COMMITS", "FILES", "LAST COMMIT"],
                    report
                        .suggestions
                        .iter()
                        .map(|suggestion| {
                            vec![
                                suggestion.name.clone(),
                                suggestion.commit_count.to_string(),
                                suggestion.files.join(", "),
                                suggestion
                                    .last_commit_on
                                    .clone()
                                    .unwrap_or_else(|| "-".into()),
                            ]
                        })
                        .collect(),
                );
            }
        }
    }
    Ok(())
}
