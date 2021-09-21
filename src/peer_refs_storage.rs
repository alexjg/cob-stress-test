use cob::{ObjectId, ObjectRefs, RefsStorage, TypeName};
use link_identities::git::Urn;
use link_crypto::PeerId;
use thiserror::Error;

use std::{collections::HashMap, str::FromStr};

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

    fn type_references<'b>(
        &'b self,
        identity_urn: &Urn,
        typename: &TypeName,
    ) -> Result<HashMap<ObjectId, ObjectRefs<'b>>, Self::Error> {
        let peer_regex_str = format!(
            r"refs/namespaces/{}/refs/remotes/([0-9a-zA-Z]+)/cob/{}/([0-9a-f]{{40}})",
            identity_urn.encode_id(),
            typename.to_string(),
        );
        let peer_regex = regex::Regex::new(peer_regex_str.as_str()).unwrap();
        let mut result = HashMap::new();

        for reference in self.repo.references().into_iter().flatten() {
            let reference = reference?;
            if let Some(name) = reference.name() {
                if let Some(caps) = peer_regex.captures(name) {
                    let oid = ObjectId::from_str(&caps[2]).unwrap();
                    let mut refs = result.entry(oid).or_insert_with(|| ObjectRefs{
                        local: None,
                        remote: Vec::new(),
                    });
                    let peer = PeerId::from_str(&caps[1]).unwrap();
                    if peer == self.peer {
                        refs.local = Some(reference);
                    } else {
                        refs.remote.push(reference);
                    }
                }
            }
        }
        Ok(result)
    }

    fn object_references<'b>(
        &'b self,
        identity_urn: &Urn,
        typename: &TypeName,
        oid: &ObjectId,
    ) -> Result<ObjectRefs<'b>, Self::Error> {
        let local_str = format!(
            "refs/namespaces/{}/refs/remotes/{}/cob/{}/{}",
            identity_urn.encode_id(),
            self.peer.default_encoding(),
            typename.to_string(),
            oid.to_string()
        );
        let local = match self.repo.find_reference(local_str.as_str()) {
            Ok(r) => Some(r),
            Err(e) if e.code() == git2::ErrorCode::NotFound => None,
            Err(e) => return Err(e.into()),
        };
        let remote_glob = globset::Glob::new(
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
        let remote = references_glob(self.repo, local_str, remote_glob)?
            .collect::<Result<Vec<git2::Reference<'_>>, Self::Error>>()?;
        Ok(ObjectRefs { local, remote })
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
    skip_ref: String,
    glob: globset::GlobMatcher,
) -> Result<ReferencesGlob<'_>, Error> {
    Ok(ReferencesGlob {
        iter: repo.references()?,
        skip: skip_ref,
        glob,
    })
}

// Copied from librad
pub struct ReferencesGlob<'a> {
    iter: git2::References<'a>,
    skip: String,
    glob: globset::GlobMatcher,
}

impl<'a> Iterator for ReferencesGlob<'a> {
    type Item = Result<git2::Reference<'a>, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        for reference in &mut self.iter {
            match reference {
                Ok(reference) => match reference.name() {
                    Some(name) if name == self.skip.as_str() => continue,
                    Some(name) if self.glob.is_match(name) => return Some(Ok(reference)),
                    _ => continue,
                },

                Err(e) => return Some(Err(e.into())),
            }
        }
        None
    }
}
