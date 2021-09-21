use super::downloaded_issue::DownloadedIssue;
use super::RepoName;

use futures::stream::StreamExt;
use thiserror::Error;
use tokio::task::JoinError;
use super::graphql;
use std::sync::Arc;

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Octocrab(#[from] octocrab::Error),
    #[error(transparent)]
    Join(#[from] JoinError),
    #[error(transparent)]
    Graphql(#[from] graphql::Error),
}

#[derive(Debug, Error)]
pub enum LoadError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Serde(#[from] serde_json::Error),
}

pub struct Storage {
    dir: std::path::PathBuf,
}

impl Storage {
    pub fn new(storage_dir: std::path::PathBuf) -> Result<Storage, std::io::Error> {
        let issues_dir = &storage_dir.join("issues");
        if !std::fs::try_exists(&issues_dir)? {
            std::fs::create_dir_all(&issues_dir)?;
        }
        Ok(Storage { dir: storage_dir })
    }

    /// List downloaded issues in this storage
    pub(crate) fn issues(&self) -> Result<Vec<DownloadedIssue>, LoadError> {
        if !std::fs::try_exists(&self.dir)? {
            Ok(Vec::new())
        } else {
            let mut issues = Vec::new();
            for file in std::fs::read_dir(&self.dir.join("issues"))? {
                let bytes = std::fs::read(file?.path())?;
                let issue: DownloadedIssue = serde_json::from_slice(&bytes[..])?;
                issues.push(issue)
            }
            Ok(issues)
        }
    }


    fn store(&self, issue: &DownloadedIssue) -> Result<(), std::io::Error> {
        let issue_filename = format!("{}.json", issue.number);
        let issue_path = self.dir.join("issues").join(issue_filename);
        let output = serde_json::to_vec(issue)?;
        std::fs::write(issue_path, &output)
    }
}

impl graphql::CursorCache for Arc<Storage> {
    fn save_cursor(&self, cursor: String) -> Result<(), std::io::Error> {
        let cursor_path = self.dir.join("last_cursor");
        std::fs::write(cursor_path, &cursor)?;
        Ok(())
    }

    fn load_cursor(&self) -> Result<Option<String>, std::io::Error> {
        let cursor_path = self.dir.join("last_cursor");
        if std::fs::try_exists(&cursor_path)? {
            Ok(Some(std::fs::read_to_string(cursor_path)?.trim().to_string()))
        } else {
            Ok(None)
        }
    }
}

pub(crate) async fn download(
    crab: octocrab::Octocrab,
    repo: RepoName,
    storage: Storage,
) -> Result<(), Error> {
    let storage = Arc::new(storage);
    let mut stream = graphql::issues(crab, repo, Box::new(storage.clone()));
    while let Some(issue) = stream.next().await {
        storage.store(&issue?)?;
    }
    Ok(())
}

