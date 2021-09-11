use std::collections::HashMap;
use std::path::{Path, PathBuf};
use thiserror::Error;

use super::GithubUserId;
use link_crypto::PeerId;

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Serde(#[from] serde_json::Error),
}

pub struct PeerAssignments {
    peers: Vec<PeerId>,
    assignments: HashMap<GithubUserId, PeerId>,
    path: PathBuf,
}

impl PeerAssignments {
    pub(crate) fn load<'a, P: AsRef<Path>>(
        path: P,
        peers: impl Iterator<Item = &'a PeerId>,
    ) -> Result<PeerAssignments, Error> {
        let assignments = if std::fs::try_exists(&path)? {
            let bytes = std::fs::read(&path)?;
            serde_json::from_slice(&bytes)?
        } else {
            HashMap::new()
        };
        Ok(PeerAssignments {
            assignments,
            path: path.as_ref().to_path_buf(),
            peers: peers.cloned().collect(),
        })
    }

    pub(crate) fn assign(&mut self, uid: GithubUserId) -> Result<&PeerId, Error> {
        if self.assignments.contains_key(&uid) {
            return Ok(self.assignments.get(&uid).unwrap());
        }
        let next_peer = next_assignment(&self.peers, self.assignments.iter_mut());
        self.assignments.insert(uid.clone(), next_peer);
        let bytes = serde_json::to_vec(&self.assignments)?;
        std::fs::write(&self.path, bytes)?;
        Ok(self.assignments.get(&uid).unwrap())
    }
}

fn next_assignment<'a>(
    peers: &[PeerId],
    assignments: impl Iterator<Item = (&'a GithubUserId, &'a mut PeerId)>,
) -> PeerId {
    let assignment_counts: HashMap<PeerId, u64> =
        assignments.fold(HashMap::new(), |mut acc, (_, peer_id)| {
            acc.entry(*peer_id).and_modify(|e| *e += 1).or_insert(1);
            acc
        });
    let mut assigned_peer = peers[0];
    let mut min_count = *assignment_counts.get(&assigned_peer).unwrap_or(&0);
    for peer in peers {
        let count = *assignment_counts.get(peer).unwrap_or(&0);
        if count < min_count {
            assigned_peer = *peer;
            min_count = count;
        }
    }
    assigned_peer
}
