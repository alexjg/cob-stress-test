use super::downloaded_issue::DownloadedIssue;
use super::RepoName;

use futures::stream::{StreamExt, TryStreamExt};
use std::convert::TryInto;
use std::pin::Pin;
use thiserror::Error;
use tokio::task::JoinError;

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Octocrab(#[from] octocrab::Error),
    #[error(transparent)]
    Join(#[from] JoinError),
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
    pub fn new(storage_dir: std::path::PathBuf) -> Storage {
        Storage { dir: storage_dir }
    }

    /// List downloaded issues in this storage
    pub(crate) fn issues(&self) -> Result<Vec<DownloadedIssue>, LoadError> {
        if !std::fs::try_exists(&self.dir)? {
            Ok(Vec::new())
        } else {
            let mut issues = Vec::new();
            for file in std::fs::read_dir(&self.dir)? {
                let bytes = std::fs::read(file?.path())?;
                let issue: DownloadedIssue = serde_json::from_slice(&bytes[..])?;
                issues.push(issue)
            }
            Ok(issues)
        }
    }

    fn stored_issues(&self) -> Vec<u64> {
        Vec::new()
    }

    fn store(&self, issue: &DownloadedIssue) -> Result<(), std::io::Error> {
        let issue_filename = format!("{}.json", issue.id);
        let issue_path = self.dir.join(issue_filename);
        let output = serde_json::to_vec(issue)?;
        std::fs::write(issue_path, &output)
    }
}

pub(crate) async fn download(
    crab: octocrab::Octocrab,
    repo: RepoName,
    storage: Storage,
) -> Result<(), Error> {
    let mut stream = issues(&crab, &repo, storage.stored_issues());
    while let Some(issue) = stream.next().await {
        storage.store(&issue?)?;
    }
    Ok(())
}

enum PaginationState<T> {
    Starting(octocrab::Octocrab),
    InProgress(octocrab::Octocrab, octocrab::Page<T>),
    Done,
}

fn issues<'a>(
    crab: &'a octocrab::Octocrab,
    reponame: &'a RepoName,
    _stored_issues: Vec<u64>,
) -> impl futures::stream::Stream<Item = Result<DownloadedIssue, Error>> + 'a {
    let stream: Pin<
        Box<
            dyn futures::Stream<
                    Item = Result<
                        Pin<
                            Box<
                                dyn futures::Stream<Item = Result<DownloadedIssue, Error>>
                                    + std::marker::Send,
                            >,
                        >,
                        Error,
                    >,
                > + std::marker::Send,
        >,
    > = futures::stream::try_unfold::<PaginationState<octocrab::models::issues::Issue>, _, _, _>(
        PaginationState::Starting(crab.clone()),
        async move |state| match state {
            PaginationState::Starting(crab) => {
                let first_page = crab
                    .issues(reponame.owner.as_str(), reponame.name.as_str())
                    .list()
                    .per_page(100)
                    .send()
                    .await?;
                Ok(Some((
                    futures::stream::empty().boxed(),
                    PaginationState::InProgress(crab, first_page),
                )))
            }
            PaginationState::Done => Ok(None),
            PaginationState::InProgress(crab, current_page) => {
                let items = futures::stream::FuturesUnordered::new();
                for issue in current_page.items {
                    items.push(get_issue(crab.clone(), reponame, issue))
                }
                let items = items.boxed();
                let next_state = crab
                    .get_page(&current_page.next)
                    .await?
                    .map(|p| PaginationState::InProgress(crab, p))
                    .unwrap_or(PaginationState::Done);
                Ok(Some((items.map_err(Error::from).boxed(), next_state)))
            }
        },
    )
    .boxed();
    stream.try_flatten().boxed()
}

async fn get_issue(
    crab: octocrab::Octocrab,
    reponame: &RepoName,
    issue: octocrab::models::issues::Issue,
) -> Result<DownloadedIssue, Error> {
    let mut comments = Vec::new();
    let mut comments_stream = get_comments(
        &crab,
        reponame.owner.as_str(),
        reponame.name.as_str(),
        issue.number.try_into().unwrap(),
    );
    while let Some(try_comments_page) = comments_stream.next().await {
        let comments_page = try_comments_page?;
        comments.extend_from_slice(&comments_page[..]);
    }
    Ok(DownloadedIssue::new(issue, comments))
}

fn get_comments<'a>(
    crab: &'a octocrab::Octocrab,
    owner: &'a str,
    repo: &'a str,
    issue: u64,
) -> impl futures::stream::Stream<
    Item = Result<Vec<octocrab::models::issues::Comment>, octocrab::Error>,
> + 'a {
    futures::stream::try_unfold::<PaginationState<octocrab::models::issues::Comment>, _, _, _>(
        PaginationState::Starting(crab.clone()),
        async move |state| match state {
            PaginationState::Done => Ok(None),
            PaginationState::Starting(crab) => {
                let first_page = crab
                    .issues(owner, repo)
                    .list_comments(issue)
                    .per_page(100)
                    .send()
                    .await?;
                Ok(Some((
                    Vec::new(),
                    PaginationState::InProgress(crab, first_page),
                )))
            }
            PaginationState::InProgress(crab, current_page) => {
                let next_state = crab
                    .get_page(&current_page.next)
                    .await?
                    .map(|p| PaginationState::InProgress(crab, p))
                    .unwrap_or(PaginationState::Done);
                Ok(Some((current_page.items, next_state)))
            }
        },
    )
    .boxed()
}

// We could do this with graphql instead, which looks like this:
//
// query getIssues($owner: String!, $name: String!) {
//   repository(owner: $owner, name: $name) {
//   	issues(first: 100) {
//       nodes {
//         author { login }
//         body
//         createdAt
//         updatedAt
//         comments(first: 100) {
//           nodes {
//             author { login }
//             body
//             createdAt
//           }
//         }
//       }
//     }
//   }
//   rateLimit {
//   	cost
//     limit
//     remaining
//   }
// }
//
// We would have to find a way of recognising when we didn't manage to get all of the comments and
// then fetch more. But this would in general be easier because we
