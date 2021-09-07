use std::collections::HashMap;
use thiserror::Error;

use link_crypto::{PeerId, SecStr, SecretKey, keystore::SecretKeyExt};

#[derive(Debug, Error)]
pub(crate) enum Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    IntoSecretKey(#[from] link_crypto::IntoSecretKeyError),
}

#[derive(Debug, Error)]
pub(crate) enum WriteError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub struct Peers(HashMap<link_crypto::PeerId, link_crypto::SecretKey>);

impl Peers {
    pub(crate) fn from_keydir<P: AsRef<std::path::Path>>(keydir: P) -> Result<Self, Error> {
        if std::fs::try_exists(&keydir)? {
            let mut keys = HashMap::new();
            for file in std::fs::read_dir(keydir)? {
                let bytes = std::fs::read(file?.path())?;
                let secbytes = SecStr::new(bytes);
                let key = SecretKey::from_bytes_and_meta(secbytes, &())?;
                let peer_id = PeerId::from(&key);
                keys.insert(peer_id, key);
            }
            Ok(Peers(keys))
        } else {
            std::fs::create_dir_all(&keydir)?;
            let mut keys = HashMap::new();
            for _ in 0..10 {
                let key = SecretKey::new();
                let peer_id = link_crypto::PeerId::from(&key);
                let filename = keydir.as_ref().join(peer_id.to_string());
                std::fs::write(filename, &key)?;
                keys.insert(peer_id, key);
            }
            Ok(Peers(keys))
        }
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item=(&PeerId, &SecretKey)> {
        self.0.iter()
    }

    pub(crate) fn get(&self, peer_id: &PeerId) -> Option<&SecretKey> {
        self.0.get(peer_id)
    }

    pub(crate) fn some_peer(&self) -> &PeerId {
        self.0.iter().next().unwrap().0
    }
}

