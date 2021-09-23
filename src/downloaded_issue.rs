use chrono::{DateTime, Utc};

use crate::GithubUserId;

#[derive(Debug, serde::Deserialize, serde::Serialize)]
pub(crate) struct DownloadedIssue {
    pub id: String,
    pub number: u64,
    pub state: String,
    pub title: String,
    pub body: Option<String>,
    pub author_id: Option<GithubUserId>,
    pub comments: Vec<DownloadedComment>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
pub(crate) struct DownloadedComment {
    pub id: String,
    pub author_id: Option<GithubUserId>,
    pub body: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
}
