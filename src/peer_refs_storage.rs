use cob::{ObjectId, RefsStorage, TypeName};
use link_identities::git::Urn;
use thiserror::Error;

use std::str::FromStr;

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Git(#[from] git2::Error),
}

pub(crate) struct PeerRefsStorage<'a> {
    peer: link_crypto::PeerId,
    repo: &'a git2::Repository,
}

impl<'a> PeerRefsStorage<'a> {
    pub(crate) fn new(
        peer: link_crypto::PeerId,
        repo: &'a git2::Repository,
    ) -> PeerRefsStorage<'a> {
        PeerRefsStorage { peer, repo }
    }
}

impl<'a> RefsStorage for PeerRefsStorage<'a> {
    type Error = Error;

    fn update_ref(
        &self,
        identity_urn: &Urn,
        typename: &TypeName,
        object_id: ObjectId,
        new_commit: git2::Oid,
    ) -> Result<(), Self::Error> {
        let literef = LiteRef {
            peer: &self.peer,
            urn: identity_urn,
            typename,
            object_id,
        };
        self.repo
            .reference(literef.to_string().as_str(), new_commit, true, "new change")?;
        Ok(())
    }

    fn type_references(
        &self,
        identity_urn: &Urn,
        typename: &TypeName,
    ) -> Result<Vec<(ObjectId, git2::Reference<'_>)>, Self::Error> {
        let mut object_ref_regex = typename.to_string();
        let oid_regex_str = r"/([0-9a-f]{40})";
        object_ref_regex.push_str(oid_regex_str);
        let oid_regex = regex::Regex::new(object_ref_regex.as_str()).unwrap();
        let refs = self.repo.references()?;
        let project_urn_str = identity_urn.encode_id();
        Ok(refs
            .into_iter()
            .flatten()
            .filter_map(|reference| {
                if let Some(name) = reference.name() {
                    if name.contains(&project_urn_str) {
                        if let Some(cap) = oid_regex.captures(name) {
                            // This unwrap is fine because the regex we used ensures the string is a
                            // valid OID
                            let oid = ObjectId::from_str(&cap[1]).unwrap();
                            return Some((oid, reference));
                        }
                    }
                }
                None
            })
            .collect())
    }

    fn object_references(
        &self,
        identity_urn: &Urn,
        typename: &TypeName,
        oid: &ObjectId,
    ) -> Result<Vec<git2::Reference<'_>>, Self::Error> {
        let glob = globset::Glob::new(
            format!(
                "refs/namespaces/{}/refs/remotes/**/cob/{}/{}",
                identity_urn.encode_id(),
                typename.to_string(),
                oid.to_string(),
            )
            .as_str(),
        )
        .unwrap()
        .compile_matcher();
        references_glob(self.repo, glob)?.collect()
    }
}

struct LiteRef<'a> {
    peer: &'a link_crypto::PeerId,
    urn: &'a Urn,
    typename: &'a TypeName,
    object_id: ObjectId,
}

impl<'a> std::fmt::Display for LiteRef<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "refs/namespaces/{}/refs/remotes/{}/cob/{}/{}",
            self.urn.encode_id(),
            self.peer,
            self.typename,
            self.object_id
        )
    }
}

fn references_glob(
    repo: &git2::Repository,
    glob: globset::GlobMatcher,
) -> Result<ReferencesGlob<'_>, Error> {
    Ok(ReferencesGlob {
        iter: repo.references()?,
        glob,
    })
}

// Copied from librad
pub struct ReferencesGlob<'a> {
    iter: git2::References<'a>,
    glob: globset::GlobMatcher,
}

impl<'a> Iterator for ReferencesGlob<'a> {
    type Item = Result<git2::Reference<'a>, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        for reference in &mut self.iter {
            match reference {
                Ok(reference) => match reference.name() {
                    Some(name) if self.glob.is_match(name) => return Some(Ok(reference)),
                    _ => continue,
                },

                Err(e) => return Some(Err(e.into())),
            }
        }
        None
    }
}
