use chrono::{DateTime, Utc};

#[derive(serde::Deserialize, serde::Serialize)]
pub(crate) struct DownloadedIssue {
    pub id: u64,
    state: String,
    title: String,
    body: Option<String>,
    author_id: u64,
    comments: Vec<DownloadedComment>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
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
}

#[derive(serde::Deserialize, serde::Serialize)]
pub(crate) struct DownloadedComment {
    id: u64,
    author_id: u64,
    body: String,
    created_at: DateTime<Utc>,
    updated_at: Option<DateTime<Utc>>,
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
