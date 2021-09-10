use automerge::LocalChange;
use either::Either;
use lazy_static::lazy_static;
use link_identities::delegation::Indirect;
use std::path::PathBuf;
use std::str::FromStr;
use thiserror::Error;

use link_identities::{
    git::{
        error::{Load as IdentityLoadError, Store as IdentityStoreError},
        Urn,
    },
    payload::{Project as ProjectSubject, ProjectPayload},
    Identities, Project,
};

use crate::downloaded_issue::DownloadedComment;

use super::downloaded_issue::DownloadedIssue;
use super::peer_assignments::{Error as PeerAssignmentsError, PeerAssignments};
use super::peer_identities::{Error as PeerIdentitiesError, PeerIdentities};
use super::peer_refs_storage::{Error as PeerRefsError, PeerRefsStorage};
use super::peers::{Error as PeersError, Peers};

lazy_static! {
    static ref SCHEMA: serde_json::Value = {
        let raw = include_bytes!("./schema.json");
        let as_json: serde_json::Value = serde_json::from_slice(raw).unwrap();
        jsonschema::JSONSchema::compile(&as_json).unwrap();
        as_json
    };
    static ref TYPENAME: cob::TypeName =
        cob::TypeName::from_str("xyz.radicle.githubissue").unwrap();
}

#[derive(Debug, Error)]
pub(crate) enum Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Peers(#[from] PeersError),
    #[error(transparent)]
    Git(#[from] git2::Error),
    #[error(transparent)]
    Serde(#[from] serde_json::Error),
    #[error(transparent)]
    PeerAssignments(#[from] PeerAssignmentsError),
    #[error(transparent)]
    PeerIdentities(#[from] PeerIdentitiesError),
    #[error(transparent)]
    IdentityLoad(#[from] IdentityLoadError),
    #[error(transparent)]
    IdentityStore(#[from] IdentityStoreError),
}

#[derive(Debug, Error)]
pub(crate) enum ImportError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Git(#[from] git2::Error),
    #[error(transparent)]
    Serde(#[from] serde_json::Error),
    #[error(transparent)]
    PeerAssignments(#[from] PeerAssignmentsError),
    #[error(transparent)]
    CobCreate(#[from] cob::error::Create<PeerRefsError>),
    #[error(transparent)]
    CobUpdate(#[from] cob::error::Update<PeerRefsError>),
}

#[derive(Debug, Error)]
pub(crate) enum ListError {
    #[error(transparent)]
    CobRetrieve(#[from] cob::error::Retrieve<PeerRefsError>),
}

pub struct LiteMonorepo {
    _root: PathBuf,
    project: Project,
    peers: Peers,
    repo: git2::Repository,
    peer_assignments: PeerAssignments,
    peer_identities: PeerIdentities,
}

impl LiteMonorepo {
    pub(crate) fn from_root<P: AsRef<std::path::Path>>(root: P) -> Result<LiteMonorepo, Error> {
        if !std::fs::try_exists(&root)? {
            std::fs::create_dir_all(&root)?;
        }
        let peers = Peers::from_keydir(&root.as_ref().join("peers"))?;
        let repo_dir = &root.as_ref().join("git");
        let repo = if !std::fs::try_exists(&repo_dir)? {
            std::fs::create_dir_all(repo_dir)?;
            git2::Repository::init_bare(repo_dir)?
        } else {
            git2::Repository::open_bare(repo_dir)?
        };
        let peer_map_path = &root.as_ref().join("peer_map");
        let peer_assignments = PeerAssignments::load(peer_map_path, peers.iter().map(|(p, _)| p))?;

        let peer_identities_path = &root.as_ref().join("peer_identities");
        let peer_identities = PeerIdentities::load(peer_identities_path, &repo, peers.iter())?;

        let project_id_path = &root.as_ref().join("project_oid");
        let identities: Identities<'_, Project> = (&repo).into();
        let project = if std::fs::try_exists(&project_id_path)? {
            let project_oid_bytes: Vec<u8> = std::fs::read(&project_id_path)?;
            let project_oid: radicle_git_ext::Oid = serde_json::from_slice(&project_oid_bytes)?;
            identities.get(project_oid.into())?
        } else {
            let key = peer_identities.some_key();
            let project = identities.create(
                ProjectPayload::new(ProjectSubject {
                    name: "theproject".into(),
                    description: None,
                    default_branch: None,
                }),
                Indirect::try_from_iter(peer_identities.keys().map(|k| Either::Left(k.public())))
                    .unwrap(),
                &key,
            )?;
            let project_oid_bytes = serde_json::to_vec(&project.content_id)?;
            std::fs::write(&project_id_path, project_oid_bytes)?;
            project
        };

        Ok(LiteMonorepo {
            _root: root.as_ref().to_path_buf(),
            peers,
            repo,
            peer_assignments,
            peer_identities,
            project,
        })
    }

    pub(crate) fn import_issue(&mut self, issue: &DownloadedIssue) -> Result<(), ImportError> {
        let creator_id = self.peer_assignments.assign(issue.author_id())?;
        let (creator_person, creator_key) = self.peer_identities.get(&creator_id).unwrap();
        let init_change = init_issue_change(issue, &creator_person.urn());
        let storage = PeerRefsStorage::new(creator_id.clone(), &self.repo);
        let mut object = cob::create_object(
            &storage,
            &self.repo,
            &(creator_key.clone()).into(),
            creator_person.content_id.into(),
            Either::Right(self.project.clone()),
            cob::NewObjectSpec {
                history: init_change,
                message: None,
                typename: TYPENAME.to_string(),
                schema_json: SCHEMA.clone(),
            },
        )?;

        for comment in &issue.comments {
            let commentor_id = self.peer_assignments.assign(comment.author_id())?;
            let (commentor_person, commentor_key) =
                self.peer_identities.get(&commentor_id).unwrap();
            let storage = PeerRefsStorage::new(commentor_id.clone(), &self.repo);
            object = cob::update_object(
                &storage,
                &(commentor_key.clone()).into(),
                &self.repo,
                commentor_person.content_id.into(),
                Either::Right(self.project.clone()),
                cob::UpdateObjectSpec {
                    object_id: object.id().clone(),
                    typename: TYPENAME.clone(),
                    message: None,
                    changes: add_comment_change(comment, &commentor_person.urn(), object.history()),
                },
            )?;
        }
        Ok(())
    }

    pub(crate) fn list_issues(&self) -> Result<usize, ListError> {
        let some_peer = self.peers.some_peer();
        let storage = PeerRefsStorage::new(some_peer.clone(), &self.repo);
        let objs = cob::retrieve_objects(
            &storage,
            &self.repo,
            Either::Right(self.project.clone()),
            &TYPENAME,
        )?;
        Ok(objs.len())
    }
}

fn init_issue_change(issue: &DownloadedIssue, author_urn: &Urn) -> cob::History {
    let mut doc = automerge::Frontend::new();
    let mut backend = automerge::Backend::new();
    let (_, change) = doc
        .change::<_, _, automerge::InvalidChangeRequest>(None, |d| {
            d.add_change(LocalChange::set(
                automerge::Path::root().key("author_urn"),
                automerge::Value::Primitive(automerge::Primitive::Str(
                    author_urn.to_string().into(),
                )),
            ))?;
            d.add_change(LocalChange::set(
                automerge::Path::root().key("title"),
                to_text(issue.title.as_str()),
            ))?;
            if let Some(body) = &issue.body {
                d.add_change(LocalChange::set(
                    automerge::Path::root().key("body"),
                    to_text(body.as_str()),
                ))?;
            }
            d.add_change(LocalChange::set(
                automerge::Path::root().key("created_at"),
                automerge::Value::Primitive(automerge::Primitive::Str(
                    issue.created_at.to_rfc3339().into(),
                )),
            ))?;
            d.add_change(LocalChange::set(
                automerge::Path::root().key("comments"),
                automerge::Value::List(Vec::new()),
            ))?;
            Ok(())
        })
        .unwrap();
    let (_, change) = backend.apply_local_change(change.unwrap()).unwrap();
    cob::History::Automerge(change.raw_bytes().to_vec())
}

fn add_comment_change(
    comment: &DownloadedComment,
    commentor_urn: &Urn,
    previous_history: &cob::History,
) -> cob::History {
    let mut frontend = automerge::Frontend::new();
    let mut backend = automerge::Backend::new();
    let cob::History::Automerge(hist) = previous_history;
    let changes: Vec<automerge::Change> = automerge::Change::load_document(&hist).unwrap();
    let patch = backend.apply_changes(changes).unwrap();
    frontend.apply_patch(patch).unwrap();

    let (_, change) = frontend
        .change::<_, _, automerge::InvalidChangeRequest>(None, |d| {
            d.add_change(LocalChange::set(
                automerge::Path::root().key("commenter_urn"),
                automerge::Value::Primitive(automerge::Primitive::Str(
                    commentor_urn.to_string().into(),
                )),
            ))?;
            d.add_change(LocalChange::set(
                automerge::Path::root().key("comment"),
                to_text(comment.body.as_str()),
            ))?;
            d.add_change(LocalChange::set(
                automerge::Path::root().key("created_at"),
                automerge::Value::Primitive(automerge::Primitive::Str(
                    comment.created_at.to_rfc3339().into(),
                )),
            ))?;
            Ok(())
        })
        .unwrap();
    let (_, change) = backend.apply_local_change(change.unwrap()).unwrap();
    cob::History::Automerge(change.raw_bytes().to_vec())
}

fn to_text(s: &str) -> automerge::Value {
    automerge::Value::Text(s.chars().map(|c| c.to_string().into()).collect())
}
