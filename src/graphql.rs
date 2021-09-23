use serde::Deserialize;
use thiserror::Error;

use futures::{StreamExt, TryStreamExt};
use std::pin::Pin;

use crate::{
    downloaded_issue::{DownloadedComment, DownloadedIssue},
    GithubUserId, RepoName,
};

static ISSUES_QUERY: &str = include_str!("./get_issues.graphql");
static ISSUE_COMMENTS_QUERY: &str = include_str!("./get_issue_comments.graphql");

#[derive(Clone, Debug, Deserialize)]
struct GithubUserLoginWrapper {
    login: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PageInfo {
    has_next_page: bool,
    end_cursor: Option<String>,
    start_cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphqlIssues {
    nodes: Vec<GraphqlIssue>,
    page_info: PageInfo,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphqlIssue {
    author: Option<GithubUserLoginWrapper>,
    number: u64,
    title: String,
    id: String,
    body: Option<String>,
    state: String,
    created_at: chrono::DateTime<chrono::Utc>,
    comments: GraphqlComments,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphqlComments {
    nodes: Vec<GraphqlComment>,
    page_info: PageInfo,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphqlComment {
    author: Option<GithubUserLoginWrapper>,
    id: String,
    body: String,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Deserialize)]
struct GraphqlIssuesRepositoryWrapper {
    repository: GraphqlIssuesWrapper,
}

#[derive(Debug, Deserialize)]
struct GraphqlIssuesWrapper {
    issues: GraphqlIssues,
}

#[derive(Debug, Deserialize)]
struct GraphqlCommentsRepositoryWrapper {
    repository: GraphqlCommentsIssueWrapper,
}

#[derive(Debug, Deserialize)]
struct GraphqlCommentsIssueWrapper {
    issue: GraphqlIssueComments,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphqlIssueComments {
    comments: GraphqlComments,
}

#[derive(Debug, Deserialize)]
struct DataWrapper<T> {
    data: T,
}

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Serde(#[from] serde_json::Error),
    #[error(transparent)]
    Octo(#[from] octocrab::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

type IssueStreamResult<'a> = Result<
    Pin<Box<dyn futures::Stream<Item = Result<DownloadedIssue, Error>> + std::marker::Send + 'a>>,
    Error,
>;

struct IssuesStreamState {
    crab: octocrab::Octocrab,
    repo: RepoName,
    cursor_cache: Box<dyn CursorCache + Send>,
}

enum PaginationState {
    Starting(IssuesStreamState),
    ProcessingPage(IssuesStreamState, Box<GraphqlIssues>),
    SavingProgress {
        state: IssuesStreamState,
        last_processed_cursor: Option<String>,
        next_cursor: Option<String>,
    },
    Done,
}

pub(crate) trait CursorCache {
    fn save_cursor(&self, cursor: String) -> Result<(), std::io::Error>;
    fn load_cursor(&self) -> Result<Option<String>, std::io::Error>;
}

pub(crate) fn issues(
    crab: octocrab::Octocrab,
    repo: RepoName,
    cursor_cache: Box<dyn CursorCache + Send>,
) -> impl futures::stream::Stream<Item = Result<DownloadedIssue, Error>> {
    let stream: Pin<Box<dyn futures::Stream<Item = IssueStreamResult> + std::marker::Send>> =
        futures::stream::try_unfold::<PaginationState, _, _, _>(
            PaginationState::Starting(IssuesStreamState {
                crab,
                repo,
                cursor_cache,
            }),
            async move |state| match state {
                PaginationState::Starting(state) => {
                    let after = state.cursor_cache.load_cursor()?;
                    println!("Getting after: {:?}", after);
                    let vars = serde_json::json!({
                        "owner": state.repo.owner,
                        "name": state.repo.name,
                        "after": after
                    });
                    let first_page: DataWrapper<GraphqlIssuesRepositoryWrapper> =
                        graphql_request(&state.crab, ISSUES_QUERY, vars).await?;
                    Ok(Some((
                        futures::stream::empty().boxed(),
                        PaginationState::ProcessingPage(
                            state,
                            Box::new(first_page.data.repository.issues),
                        ),
                    )))
                }
                PaginationState::Done => Ok(None),
                PaginationState::ProcessingPage(state, current_page) => {
                    let items = futures::stream::FuturesUnordered::new();
                    for issue in current_page.nodes {
                        items.push(get_issue(state.crab.clone(), state.repo.clone(), issue))
                    }
                    let items = items.boxed();
                    let next_state = if current_page.page_info.has_next_page {
                        PaginationState::SavingProgress {
                            state,
                            last_processed_cursor: current_page.page_info.start_cursor,
                            next_cursor: current_page.page_info.end_cursor,
                        }
                    } else {
                        PaginationState::Done
                    };
                    Ok(Some((items.map_err(Error::from).boxed(), next_state)))
                }
                PaginationState::SavingProgress {
                    state,
                    last_processed_cursor,
                    next_cursor,
                } => {
                    if let Some(last) = last_processed_cursor {
                        state.cursor_cache.save_cursor(last)?;
                    }
                    let next_state = if let Some(end) = next_cursor {
                        let vars = serde_json::json!({
                            "owner": state.repo.owner,
                            "name": state.repo.name,
                            "after": end
                        });
                        let next_page: DataWrapper<GraphqlIssuesRepositoryWrapper> =
                            graphql_request(&state.crab, ISSUES_QUERY, vars).await?;
                        PaginationState::ProcessingPage(
                            state,
                            Box::new(next_page.data.repository.issues),
                        )
                    } else {
                        PaginationState::Done
                    };
                    Ok(Some((futures::stream::empty().boxed(), next_state)))
                }
            },
        )
        .boxed();
    stream.try_flatten().boxed()
}

async fn get_issue(
    crab: octocrab::Octocrab,
    repo: RepoName,
    issue: GraphqlIssue,
) -> Result<DownloadedIssue, Error> {
    let comments = comments(crab, repo, &issue).await?;
    Ok(issue.into_downloaded(comments))
}

async fn comments(
    crab: octocrab::Octocrab,
    repo: RepoName,
    issue: &GraphqlIssue,
) -> Result<Vec<DownloadedComment>, Error> {
    let mut page = issue.comments.page_info.clone();
    let mut comments: Vec<DownloadedComment> =
        issue.comments.nodes.iter().map(|c| c.into()).collect();
    while page.has_next_page {
        println!("loading additional comments for {}", issue.number);
        let vars = serde_json::json!({
            "owner": repo.owner,
            "name": repo.name,
            "number": issue.number,
            "after": page.end_cursor
        });
        let next_page: DataWrapper<GraphqlCommentsRepositoryWrapper> =
            match graphql_request(&crab, ISSUE_COMMENTS_QUERY, vars).await {
                Ok(p) => p,
                Err(e) => {
                    println!("Error whilst fetching comments for {}", issue.number);
                    return Err(e.into());
                }
            };
        comments.extend(
            next_page
                .data
                .repository
                .issue
                .comments
                .nodes
                .iter()
                .map(|c| c.into()),
        );
        page = next_page.data.repository.issue.comments.page_info;
    }
    Ok(comments)
}

async fn graphql_request<R: octocrab::FromResponse>(
    crab: &octocrab::Octocrab,
    query: &'static str,
    variables: serde_json::Value,
) -> Result<R, octocrab::Error> {
    crab.post(
        "graphql",
        Some(&serde_json::json! {{
            "query": query,
            "variables": variables
        }}),
    )
    .await
}

impl From<GithubUserLoginWrapper> for GithubUserId {
    fn from(w: GithubUserLoginWrapper) -> Self {
        GithubUserId(w.login)
    }
}

impl From<&GraphqlComment> for DownloadedComment {
    fn from(c: &GraphqlComment) -> Self {
        DownloadedComment {
            body: c.body.clone(),
            id: c.id.clone(),
            author_id: c.author.clone().map(|a| a.into()),
            created_at: c.created_at,
            updated_at: c.updated_at,
        }
    }
}

impl GraphqlIssue {
    fn into_downloaded(self, comments: Vec<DownloadedComment>) -> DownloadedIssue {
        DownloadedIssue {
            author_id: self.author.map(|a| a.into()),
            id: self.id,
            body: self.body,
            comments,
            number: self.number,
            state: self.state,
            created_at: self.created_at,
            title: self.title,
        }
    }
}
