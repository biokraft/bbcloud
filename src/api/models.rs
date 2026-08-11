use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub struct User {
    pub uuid: Option<String>,
    pub display_name: Option<String>,
    pub nickname: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct BranchName {
    pub name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct Endpoint {
    pub branch: Option<BranchName>,
}

#[derive(Debug, Deserialize)]
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
