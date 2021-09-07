use chrono::{DateTime, Utc};

use crate::GithubUserId;

#[derive(serde::Deserialize, serde::Serialize)]
pub(crate) struct DownloadedIssue {
    pub id: u64,
    pub state: String,
    pub title: String,
    pub body: Option<String>,
    pub author_id: u64,
    pub comments: Vec<DownloadedComment>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl DownloadedIssue {
    pub(crate) fn new(gh_issue: octocrab::models::issues::Issue, comments: Vec<octocrab::models::issues::Comment>) -> DownloadedIssue {
        DownloadedIssue {
            id: gh_issue.id.0,
            state: gh_issue.state,
            title: gh_issue.title,
            body: gh_issue.body,
            author_id: gh_issue.user.id.0,
            comments: comments.iter().map(|c| c.into()).collect(),
            created_at: gh_issue.created_at,
            updated_at: gh_issue.updated_at,
        }
    }

    pub fn author_id(&self) -> GithubUserId {
        GithubUserId(self.author_id)
    }
}

#[derive(serde::Deserialize, serde::Serialize)]
pub(crate) struct DownloadedComment {
    pub id: u64,
    pub author_id: u64,
    pub body: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
}

impl DownloadedComment {
    pub(crate) fn author_id(&self) -> GithubUserId {
        GithubUserId(self.author_id)
    }
}

impl From<&octocrab::models::issues::Comment> for DownloadedComment {
    fn from(c: &octocrab::models::issues::Comment) -> Self {
        DownloadedComment {
            id: c.id.0,
            author_id: c.user.id.0,
            body: c.body.clone().unwrap_or_else(|| "".to_string()),
            created_at: c.created_at,
            updated_at: c.updated_at,
        }
    }
}
