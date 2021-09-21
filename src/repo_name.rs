use std::str::FromStr;
use thiserror::Error;

#[derive(Debug, Error)]
#[error("Repository name must be <organisation/owner>")]
pub struct ParseError {}

#[derive(Clone)]
pub(crate) struct RepoName {
    pub(crate) owner: String,
    pub(crate) name: String,
}

impl FromStr for RepoName {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let components: Vec<&str> = s.split('/').collect();
        match &components[..] {
            [owner, repo] => Ok(RepoName {
                owner: owner.to_string(),
                name: repo.to_string(),
            }),
            _ => Err(ParseError {}),
        }
    }
}

impl std::fmt::Display for RepoName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.owner, self.name)
    }
}
