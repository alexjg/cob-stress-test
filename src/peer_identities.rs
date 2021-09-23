use std::collections::HashMap;

use thiserror::Error;

use link_crypto::{PeerId, PublicKey, SecretKey};
use link_identities::{
    delegation::Direct,
    git::error::{Load, Store},
    payload::{Person as PersonSubject, PersonPayload},
    Person,
};

#[derive(Debug, Error)]
pub(crate) enum Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Serde(#[from] serde_json::Error),
    #[error(transparent)]
    LoadIdentity(#[from] Load),
    #[error(transparent)]
    StoreIdentity(#[from] Store),
    #[error("missing peer identity in monorepo for {peer}")]
    MissingPeer { peer: PeerId },
}

pub(crate) struct PeerIdentities(HashMap<PeerId, (Person, SecretKey)>);

impl PeerIdentities {
    pub(crate) fn load<'a, P: AsRef<std::path::Path>>(
        index_path: P,
        repo: &git2::Repository,
        peers: impl Iterator<Item = (&'a PeerId, &'a SecretKey)>,
    ) -> Result<PeerIdentities, Error> {
        let identities: link_identities::Identities<'_, Person> = repo.into();
        let mut ids: HashMap<PeerId, (Person, SecretKey)> = HashMap::new();
        if std::fs::try_exists(&index_path)? {
            let key_by_peer: HashMap<PeerId, SecretKey> =
                peers.map(|(p, s)| (*p, s.clone())).collect();
            let bytes = std::fs::read(&index_path)?;
            let mapping: HashMap<PeerId, radicle_git_ext::Oid> = serde_json::from_slice(&bytes)?;
            for (peer, oid) in mapping {
                let identity = identities.get(oid.into())?;
                let key = key_by_peer.get(&peer).ok_or(Error::MissingPeer { peer })?;
                ids.insert(peer, (identity, key.clone()));
            }
        } else {
            for (peer, key) in peers {
                let payload: PersonPayload = PersonPayload::new(PersonSubject {
                    name: peer.to_string().into(),
                });
                let pubkey: PublicKey = key.public();
                let delegations: Direct = Direct::new(pubkey);
                let identity = identities.create(payload, delegations, key)?;
                ids.insert(*peer, (identity, key.clone()));
            }
            let oid_mapping: HashMap<&PeerId, radicle_git_ext::Oid> =
                ids.iter().map(|(p, (id, _))| (p, id.content_id)).collect();
            let bytes = serde_json::to_vec(&oid_mapping)?;
            std::fs::write(&index_path, &bytes)?;
        }
        Ok(PeerIdentities(ids))
    }

    pub(crate) fn some_key(&self) -> SecretKey {
        self.0.values().next().unwrap().1.clone()
    }

    pub(crate) fn get(&self, peer_id: &PeerId) -> Option<&(Person, SecretKey)> {
        self.0.get(peer_id)
    }

    pub(crate) fn keys(&self) -> impl Iterator<Item = &SecretKey> {
        self.0.values().map(|v| &v.1)
    }
}
