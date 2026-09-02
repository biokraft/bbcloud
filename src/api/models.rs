use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub struct User {
    pub uuid: Option<String>,
    pub account_id: Option<String>,
    pub display_name: Option<String>,
    pub nickname: Option<String>,
}

impl User {
    /// The name a human recognizes. `display_name` is what the Bitbucket web ui
    /// shows, so it is preferred over the nickname. A bare `uuid` (the `{uuid}`
    /// escape hatch has no names at all) is the next best identifier — it still
    /// names somebody, unlike the final `"-"` fallback.
    pub fn name(&self) -> &str {
        self.display_name
            .as_deref()
            .or(self.nickname.as_deref())
            .or(self.uuid.as_deref())
            .unwrap_or("-")
    }
}

#[derive(Debug, Deserialize)]
pub struct BranchName {
    pub name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct Endpoint {
    pub branch: Option<BranchName>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Link {
    pub href: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct Links {
    pub html: Option<Link>,
}

#[derive(Debug, Deserialize)]
pub struct Participant {
    pub user: Option<User>,
    pub state: Option<String>,
    pub role: Option<String>,
    #[serde(default)]
    pub approved: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewState {
    Approved,
    ChangesRequested,
    Pending,
}

impl ReviewState {
    pub fn from_api(state: Option<&str>) -> Self {
        match state {
            Some("approved") => Self::Approved,
            Some("changes_requested") => Self::ChangesRequested,
            _ => Self::Pending,
        }
    }

    pub fn mark(self) -> &'static str {
        match self {
            Self::Approved => "✓",
            Self::ChangesRequested => "✗",
            Self::Pending => "·",
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Approved => "approved",
            Self::ChangesRequested => "changes_requested",
            Self::Pending => "pending",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ReviewerState {
    pub name: String,
    pub uuid: Option<String>,
    pub state: ReviewState,
}

/// One entry from `…/pullrequests/{id}/statuses`. Every field is optional
/// because a reporter may omit any of them, and a missing name must not cost
/// us the row.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BuildStatus {
    pub key: Option<String>,
    pub name: Option<String>,
    pub state: Option<String>,
    pub url: Option<String>,
}

/// A pull request can carry one status per reporting tool, so the table needs a
/// single word. `None` covers both "no checks reported" and "a state this
/// version does not recognise" — an unknown future state must never fail a list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum BuildState {
    Failed,
    Stopped,
    InProgress,
    Successful,
    None,
}

impl BuildState {
    pub fn from_api(state: Option<&str>) -> Self {
        match state.map(str::to_ascii_uppercase).as_deref() {
            Some("FAILED") => Self::Failed,
            Some("STOPPED") => Self::Stopped,
            Some("INPROGRESS") => Self::InProgress,
            Some("SUCCESSFUL") => Self::Successful,
            _ => Self::None,
        }
    }

    /// Worst first. Used only for the rollup ordering.
    pub fn rank(self) -> u8 {
        match self {
            Self::Failed => 0,
            Self::Stopped => 1,
            Self::InProgress => 2,
            Self::Successful => 3,
            Self::None => 4,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Failed => "FAILED",
            Self::Stopped => "STOPPED",
            Self::InProgress => "INPROGRESS",
            Self::Successful => "SUCCESSFUL",
            Self::None => "-",
        }
    }

    /// Worst-wins: one failing check needs attention whatever else passed.
    pub fn rollup(statuses: &[BuildStatus]) -> Self {
        statuses
            .iter()
            .map(|s| Self::from_api(s.state.as_deref()))
            .min_by_key(|s| s.rank())
            .unwrap_or(Self::None)
    }
}

#[derive(Debug, Deserialize)]
pub struct PullRequest {
    pub id: u64,
    pub title: Option<String>,
    pub state: Option<String>,
    pub author: Option<User>,
    pub source: Option<Endpoint>,
    pub destination: Option<Endpoint>,
    pub links: Option<Links>,
    #[serde(default)]
    pub reviewers: Vec<User>,
    #[serde(default)]
    pub participants: Vec<Participant>,
    #[serde(default)]
    pub draft: bool,
    /// The api's own rfc3339 string, passed through unformatted: the consumer is
    /// usually an agent computing an age, and a pre-formatted "3 days ago" would
    /// throw away the precision it needs.
    pub updated_on: Option<String>,
    /// Every comment on the pull request, inline and general, replies included —
    /// bitbucket's own counter. `None` when the api did not return the field,
    /// which is not the same as zero: a caller that treats absence as "no
    /// comments" would silently hide the very activity this exists to surface,
    /// so absence must be read as "unknown, look properly".
    pub comment_count: Option<u64>,
}

/// A Bitbucket project, the container a repository lives in.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Project {
    pub key: Option<String>,
    pub name: Option<String>,
    pub uuid: Option<String>,
    pub is_private: Option<bool>,
}

impl Project {
    pub fn key_or_dash(&self) -> &str {
        self.key.as_deref().unwrap_or("-")
    }

    pub fn name_or_dash(&self) -> &str {
        self.name.as_deref().unwrap_or("-")
    }

    pub fn access(&self) -> &'static str {
        access_word(self.is_private)
    }
}

/// One entry of `links.clone[]`, which Bitbucket returns as a list tagged by
/// protocol rather than as named fields.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CloneLink {
    pub name: Option<String>,
    pub href: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RepositoryLinks {
    pub html: Option<Link>,
    #[serde(default)]
    pub clone: Option<Vec<CloneLink>>,
}

/// `is_private` rendered as a word. `false` in a column is ambiguous about
/// which way it points, so neither list command prints a bare boolean.
fn access_word(is_private: Option<bool>) -> &'static str {
    match is_private {
        Some(true) => "private",
        Some(false) => "public",
        None => "-",
    }
}

/// A repository as returned by `GET /repositories/{workspace}` and by the
/// creation endpoint. `full_name` is `"workspace/repo"`, which
/// `RepoSlug::parse` accepts directly.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Repository {
    pub full_name: Option<String>,
    pub name: Option<String>,
    pub slug: Option<String>,
    pub description: Option<String>,
    pub is_private: Option<bool>,
    pub project: Option<Project>,
    pub updated_on: Option<String>,
    pub links: Option<RepositoryLinks>,
}

impl Repository {
    /// The clone url the server reported: ssh by preference, https otherwise.
    /// Never assembled locally — a hand-built url would be wrong for a
    /// workspace on a custom domain, and a wrong clone url is worse than none.
    pub fn clone_url(&self) -> Option<&str> {
        let clones = self.links.as_ref()?.clone.as_ref()?;
        let by_name = |want: &str| {
            clones
                .iter()
                .find(|c| c.name.as_deref() == Some(want))
                .and_then(|c| c.href.as_deref())
        };
        by_name("ssh").or_else(|| by_name("https"))
    }

    pub fn html_url(&self) -> Option<&str> {
        self.links.as_ref()?.html.as_ref()?.href.as_deref()
    }

    pub fn display_name(&self) -> &str {
        self.slug
            .as_deref()
            .or(self.name.as_deref())
            .or(self.full_name.as_deref())
            .unwrap_or("-")
    }

    pub fn project_key(&self) -> &str {
        self.project
            .as_ref()
            .map(|p| p.key_or_dash())
            .unwrap_or("-")
    }

    pub fn access(&self) -> &'static str {
        access_word(self.is_private)
    }
}

impl PullRequest {
    pub fn source_branch(&self) -> &str {
        self.source
            .as_ref()
            .and_then(|e| e.branch.as_ref())
            .and_then(|b| b.name.as_deref())
            .unwrap_or("-")
    }

    pub fn destination_branch(&self) -> &str {
        self.destination
            .as_ref()
            .and_then(|e| e.branch.as_ref())
            .and_then(|b| b.name.as_deref())
            .unwrap_or("-")
    }

    pub fn html_url(&self) -> &str {
        self.links
            .as_ref()
            .and_then(|l| l.html.as_ref())
            .and_then(|l| l.href.as_deref())
            .unwrap_or("-")
    }

    pub fn author_name(&self) -> &str {
        self.author
            .as_ref()
            .and_then(|a| a.nickname.as_deref().or(a.display_name.as_deref()))
            .unwrap_or("-")
    }

    /// Who is on the hook for this pull request, and what each has decided.
    ///
    /// `reviewers[]` is the tagged set but carries no decision; `participants[]`
    /// carries the decision but also includes people who only commented. So
    /// participants with the REVIEWER role are the primary source, and anyone
    /// tagged who has not shown up there yet is appended as Pending.
    pub fn reviewer_states(&self) -> Vec<ReviewerState> {
        let mut out: Vec<ReviewerState> = self
            .participants
            .iter()
            .filter(|p| p.role.as_deref() == Some("REVIEWER"))
            .filter_map(|p| {
                p.user.as_ref().map(|u| ReviewerState {
                    name: u.name().to_string(),
                    uuid: u.uuid.clone(),
                    state: ReviewState::from_api(p.state.as_deref()),
                })
            })
            .collect();

        for reviewer in &self.reviewers {
            let already = out.iter().any(|seen| {
                match (seen.uuid.as_deref(), reviewer.uuid.as_deref()) {
                    (Some(a), Some(b)) => a == b,
                    // One side has no uuid, so the name is all there is to match on.
                    _ => seen.name == reviewer.name(),
                }
            });
            if !already {
                out.push(ReviewerState {
                    name: reviewer.name().to_string(),
                    uuid: reviewer.uuid.clone(),
                    state: ReviewState::Pending,
                });
            }
        }

        out
    }

    /// Bitbucket keeps `draft` as a boolean while `state` stays OPEN, so the two
    /// have to be folded into one word for the table.
    pub fn display_state(&self) -> String {
        if self.draft {
            return "Draft".to_string();
        }
        match self.state.as_deref() {
            Some(state) if !state.is_empty() => {
                let lower = state.to_lowercase();
                let mut chars = lower.chars();
                match chars.next() {
                    Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                    None => "-".to_string(),
                }
            }
            _ => "-".to_string(),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct CommentContent {
    pub raw: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct Inline {
    pub path: Option<String>,
    pub from: Option<u64>,
    pub to: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct CommentParent {
    pub id: u64,
}

#[derive(Debug, Deserialize)]
pub struct Comment {
    pub id: u64,
    pub content: Option<CommentContent>,
    pub user: Option<User>,
    pub created_on: Option<String>,
    pub inline: Option<Inline>,
    /// Set on a reply, holding the comment it answers.
    pub parent: Option<CommentParent>,
    #[serde(default)]
    pub deleted: bool,
    /// Present (even as `{}`) when the inline thread has been resolved.
    pub resolution: Option<serde_json::Value>,
}

impl Comment {
    pub fn is_inline(&self) -> bool {
        self.inline
            .as_ref()
            .and_then(|i| i.path.as_deref())
            .is_some_and(|p| !p.is_empty())
    }

    pub fn is_resolved(&self) -> bool {
        self.resolution.is_some()
    }

    pub fn parent_id(&self) -> Option<u64> {
        self.parent.as_ref().map(|p| p.id)
    }

    pub fn body(&self) -> String {
        if self.deleted {
            return "[deleted]".to_string();
        }
        self.content
            .as_ref()
            .and_then(|c| c.raw.clone())
            .unwrap_or_default()
    }

    pub fn author(&self) -> &str {
        self.user
            .as_ref()
            .and_then(|u| u.display_name.as_deref())
            .unwrap_or("Unknown")
    }
}

#[derive(Debug, Deserialize)]
pub struct CommitAuthor {
    pub user: Option<User>,
    pub raw: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CommitTarget {
    pub author: Option<CommitAuthor>,
    pub date: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct BranchRef {
    pub name: String,
    pub target: Option<CommitTarget>,
}

impl BranchRef {
    pub fn owner(&self) -> String {
        self.target
            .as_ref()
            .and_then(|t| t.author.as_ref())
            .and_then(|a| {
                a.user
                    .as_ref()
                    .and_then(|u| u.display_name.clone())
                    .or_else(|| a.raw.clone())
            })
            .unwrap_or_else(|| "-".to_string())
    }
}

#[derive(Debug, Deserialize)]
pub struct CommitSummary {
    pub raw: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct Commit {
    pub hash: Option<String>,
    pub summary: Option<CommitSummary>,
}

#[derive(Debug, Deserialize)]
pub struct DiffStatEntry {
    pub status: Option<String>,
    #[serde(rename = "new")]
    pub new_file: Option<PathEntry>,
    #[serde(rename = "old")]
    pub old_file: Option<PathEntry>,
}

#[derive(Debug, Deserialize)]
pub struct PathEntry {
    pub path: Option<String>,
}

impl DiffStatEntry {
    pub fn path(&self) -> &str {
        self.new_file
            .as_ref()
            .and_then(|p| p.path.as_deref())
            .or_else(|| self.old_file.as_ref().and_then(|p| p.path.as_deref()))
            .unwrap_or("-")
    }
}

#[derive(Debug, Serialize)]
pub struct ReviewerRef {
    pub uuid: String,
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn pr_from(json: serde_json::Value) -> PullRequest {
        serde_json::from_value(json).expect("fixture should deserialize")
    }

    #[test]
    fn reviewer_states_reads_state_from_participants() {
        let pr = pr_from(serde_json::json!({
            "id": 1,
            "reviewers": [
                { "uuid": "{a}", "display_name": "Ana" },
                { "uuid": "{b}", "display_name": "Bo" },
                { "uuid": "{c}", "display_name": "Cy" }
            ],
            "participants": [
                { "role": "REVIEWER", "state": "approved", "user": { "uuid": "{a}", "display_name": "Ana" } },
                { "role": "REVIEWER", "state": "changes_requested", "user": { "uuid": "{b}", "display_name": "Bo" } },
                { "role": "REVIEWER", "state": null, "user": { "uuid": "{c}", "display_name": "Cy" } }
            ]
        }));

        let states = pr.reviewer_states();
        assert_eq!(states.len(), 3);
        assert_eq!(states[0].name, "Ana");
        assert_eq!(states[0].state, ReviewState::Approved);
        assert_eq!(states[1].state, ReviewState::ChangesRequested);
        assert_eq!(states[2].state, ReviewState::Pending);
    }

    /// A tagged reviewer who has not opened the pull request at all is absent from
    /// `participants`. They must still be listed, or the column under-reports who is
    /// on the hook.
    #[test]
    fn reviewer_states_includes_a_reviewer_missing_from_participants() {
        let pr = pr_from(serde_json::json!({
            "id": 1,
            "reviewers": [{ "uuid": "{a}", "display_name": "Ana" }],
            "participants": []
        }));

        let states = pr.reviewer_states();
        assert_eq!(states.len(), 1);
        assert_eq!(states[0].name, "Ana");
        assert_eq!(states[0].state, ReviewState::Pending);
    }

    /// Someone who merely commented has role PARTICIPANT. Counting them as a
    /// reviewer would invent reviewers nobody tagged.
    #[test]
    fn reviewer_states_excludes_plain_participants() {
        let pr = pr_from(serde_json::json!({
            "id": 1,
            "reviewers": [],
            "participants": [
                { "role": "PARTICIPANT", "state": "approved", "user": { "uuid": "{z}", "display_name": "Zed" } }
            ]
        }));

        assert!(pr.reviewer_states().is_empty());
    }

    /// The same person appears in both arrays; they must be listed once, with the
    /// participant state rather than a duplicate Pending row.
    #[test]
    fn reviewer_states_does_not_duplicate_across_both_arrays() {
        let pr = pr_from(serde_json::json!({
            "id": 1,
            "reviewers": [{ "uuid": "{a}", "display_name": "Ana" }],
            "participants": [
                { "role": "REVIEWER", "state": "approved", "user": { "uuid": "{a}", "display_name": "Ana" } }
            ]
        }));

        let states = pr.reviewer_states();
        assert_eq!(states.len(), 1);
        assert_eq!(states[0].state, ReviewState::Approved);
    }

    /// Dedup must survive a missing uuid on one side by falling back to the name.
    #[test]
    fn reviewer_states_dedups_by_name_when_a_uuid_is_absent() {
        let pr = pr_from(serde_json::json!({
            "id": 1,
            "reviewers": [{ "display_name": "Ana" }],
            "participants": [
                { "role": "REVIEWER", "state": "approved", "user": { "display_name": "Ana" } }
            ]
        }));

        assert_eq!(pr.reviewer_states().len(), 1);
    }

    #[test]
    fn marks_are_stable_glyphs() {
        assert_eq!(ReviewState::Approved.mark(), "✓");
        assert_eq!(ReviewState::ChangesRequested.mark(), "✗");
        assert_eq!(ReviewState::Pending.mark(), "·");
    }

    #[test]
    fn review_state_serializes_in_snake_case() {
        let json = serde_json::to_string(&ReviewState::ChangesRequested).unwrap();
        assert_eq!(json, "\"changes_requested\"");
    }

    #[test]
    fn draft_wins_over_open_state() {
        let pr = pr_from(serde_json::json!({ "id": 1, "state": "OPEN", "draft": true }));
        assert_eq!(pr.display_state(), "Draft");
    }

    #[test]
    fn display_state_title_cases_the_api_value() {
        let pr = pr_from(serde_json::json!({ "id": 1, "state": "DECLINED" }));
        assert_eq!(pr.display_state(), "Declined");
    }

    #[test]
    fn display_state_without_a_state_is_a_dash() {
        let pr = pr_from(serde_json::json!({ "id": 1 }));
        assert_eq!(pr.display_state(), "-");
    }

    #[test]
    fn user_name_prefers_display_name_then_nickname() {
        let full: User = serde_json::from_value(
            serde_json::json!({ "display_name": "Ana Cruz", "nickname": "ana" }),
        )
        .unwrap();
        assert_eq!(full.name(), "Ana Cruz");

        let nick_only: User =
            serde_json::from_value(serde_json::json!({ "nickname": "ana" })).unwrap();
        assert_eq!(nick_only.name(), "ana");

        let empty: User = serde_json::from_value(serde_json::json!({})).unwrap();
        assert_eq!(empty.name(), "-");
    }

    /// The `{uuid}` escape hatch has no names at all; `name()` must still
    /// identify the person rather than falling through to `"-"`.
    #[test]
    fn user_name_falls_back_to_uuid_when_no_names_are_set() {
        let uuid_only: User =
            serde_json::from_value(serde_json::json!({ "uuid": "{5f3a}" })).unwrap();
        assert_eq!(uuid_only.name(), "{5f3a}");
    }

    fn status(state: Option<&str>) -> BuildStatus {
        BuildStatus {
            key: Some("PIPELINE".into()),
            name: Some("Pipeline #1".into()),
            state: state.map(str::to_string),
            url: None,
        }
    }

    #[test]
    fn build_state_from_api_is_case_insensitive() {
        assert_eq!(
            BuildState::from_api(Some("SUCCESSFUL")),
            BuildState::Successful
        );
        assert_eq!(
            BuildState::from_api(Some("successful")),
            BuildState::Successful
        );
        assert_eq!(
            BuildState::from_api(Some("InProgress")),
            BuildState::InProgress
        );
        assert_eq!(BuildState::from_api(Some("FAILED")), BuildState::Failed);
        assert_eq!(BuildState::from_api(Some("STOPPED")), BuildState::Stopped);
    }

    #[test]
    fn build_state_from_api_degrades_on_unknown_and_missing() {
        assert_eq!(BuildState::from_api(Some("TELEPORTED")), BuildState::None);
        assert_eq!(BuildState::from_api(None), BuildState::None);
    }

    #[test]
    fn rollup_of_empty_is_none() {
        assert_eq!(BuildState::rollup(&[]), BuildState::None);
    }

    #[test]
    fn rollup_is_worst_wins() {
        // Every state loses to a failure, whichever order they arrive in.
        for other in ["SUCCESSFUL", "INPROGRESS", "STOPPED"] {
            assert_eq!(
                BuildState::rollup(&[status(Some(other)), status(Some("FAILED"))]),
                BuildState::Failed
            );
            assert_eq!(
                BuildState::rollup(&[status(Some("FAILED")), status(Some(other))]),
                BuildState::Failed
            );
        }
        assert_eq!(
            BuildState::rollup(&[status(Some("SUCCESSFUL")), status(Some("STOPPED"))]),
            BuildState::Stopped
        );
        assert_eq!(
            BuildState::rollup(&[status(Some("SUCCESSFUL")), status(Some("INPROGRESS"))]),
            BuildState::InProgress
        );
        assert_eq!(
            BuildState::rollup(&[status(Some("SUCCESSFUL")), status(Some("SUCCESSFUL"))]),
            BuildState::Successful
        );
    }

    /// An unrecognised state must not be treated as worse than everything else,
    /// or one unknown reporter would paint every pull request red.
    #[test]
    fn rollup_ignores_unknown_states_next_to_a_real_one() {
        assert_eq!(
            BuildState::rollup(&[status(None), status(Some("SUCCESSFUL"))]),
            BuildState::Successful
        );
    }

    #[test]
    fn build_state_serialises_lowercase() {
        assert_eq!(
            serde_json::to_string(&BuildState::InProgress).unwrap(),
            "\"inprogress\""
        );
        assert_eq!(
            serde_json::to_string(&BuildState::None).unwrap(),
            "\"none\""
        );
    }

    #[test]
    fn build_state_labels_match_bitbucket_wording() {
        assert_eq!(BuildState::Failed.label(), "FAILED");
        assert_eq!(BuildState::Stopped.label(), "STOPPED");
        assert_eq!(BuildState::InProgress.label(), "INPROGRESS");
        assert_eq!(BuildState::Successful.label(), "SUCCESSFUL");
        assert_eq!(BuildState::None.label(), "-");
    }

    #[test]
    fn pull_request_carries_updated_on() {
        let pr: PullRequest =
            serde_json::from_str(r#"{"id":1,"updated_on":"2026-08-10T09:00:00+00:00"}"#).unwrap();
        assert_eq!(pr.updated_on.as_deref(), Some("2026-08-10T09:00:00+00:00"));
    }

    #[test]
    fn pull_request_without_updated_on_is_none() {
        let pr: PullRequest = serde_json::from_str(r#"{"id":1}"#).unwrap();
        assert!(pr.updated_on.is_none());
    }

    #[test]
    fn repository_deserialises() {
        let repo: Repository = serde_json::from_str(r#"{"full_name":"acme/api"}"#).unwrap();
        assert_eq!(repo.full_name.as_deref(), Some("acme/api"));
    }

    #[test]
    fn repository_tolerates_a_missing_full_name() {
        let repo: Repository = serde_json::from_str(r#"{}"#).unwrap();
        assert!(repo.full_name.is_none());
    }

    #[test]
    fn repository_reads_ssh_clone_url_in_preference_to_https() {
        let json = serde_json::json!({
            "full_name": "acme/api",
            "links": { "clone": [
                { "name": "https", "href": "https://bitbucket.org/acme/api.git" },
                { "name": "ssh", "href": "git@bitbucket.org:acme/api.git" }
            ]}
        });
        let repo: Repository = serde_json::from_value(json).unwrap();
        assert_eq!(repo.clone_url(), Some("git@bitbucket.org:acme/api.git"));
    }

    #[test]
    fn repository_falls_back_to_https_clone_url() {
        let json = serde_json::json!({
            "links": { "clone": [{ "name": "https", "href": "https://bitbucket.org/acme/api.git" }] }
        });
        let repo: Repository = serde_json::from_value(json).unwrap();
        assert_eq!(repo.clone_url(), Some("https://bitbucket.org/acme/api.git"));
    }

    #[test]
    fn repository_tolerates_a_response_with_nothing_in_it() {
        // Every field is Option-tolerant by house rule, and the accessors must not
        // panic on the emptiest body the api could return.
        let repo: Repository = serde_json::from_value(serde_json::json!({})).unwrap();
        assert_eq!(repo.clone_url(), None);
        assert_eq!(repo.project_key(), "-");
        assert_eq!(repo.access(), "-");
    }

    #[test]
    fn repository_renders_privacy_as_a_word_not_a_boolean() {
        let private: Repository =
            serde_json::from_value(serde_json::json!({ "is_private": true })).unwrap();
        let public: Repository =
            serde_json::from_value(serde_json::json!({ "is_private": false })).unwrap();
        assert_eq!(private.access(), "private");
        assert_eq!(public.access(), "public");
    }

    #[test]
    fn repository_reads_its_project_key() {
        let repo: Repository = serde_json::from_value(serde_json::json!({
            "project": { "key": "ENG", "name": "Engineering" }
        }))
        .unwrap();
        assert_eq!(repo.project_key(), "ENG");
    }
}
