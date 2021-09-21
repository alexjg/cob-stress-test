use automerge::LocalChange;
use either::Either;
use lazy_static::lazy_static;
use link_identities::delegation::Indirect;
use std::str::FromStr;
use std::{collections::HashMap, path::PathBuf};

use link_identities::{
    git::Urn,
    payload::{Project as ProjectSubject, ProjectPayload},
    Identities, Project,
};

use crate::downloaded_issue::DownloadedComment;

use super::downloaded_issue::DownloadedIssue;
use super::peer_assignments::PeerAssignments;
use super::peer_identities::PeerIdentities;
use super::peer_refs_storage::PeerRefsStorage;
use super::peers::Peers;

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

mod error {
    use thiserror::Error;

    use super::super::peer_assignments::Error as PeerAssignmentsError;
    use super::super::peer_identities::Error as PeerIdentitiesError;
    use super::super::peer_refs_storage::Error as PeerRefsError;
    use super::super::peers::Error as PeersError;
    use link_identities::git::error::{Load as IdentityLoadError, Store as IdentityStoreError};

    #[derive(Debug, Error)]
    pub(crate) enum CreateOrOpen {
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
    pub(crate) enum Import {
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
    pub(crate) enum List {
        #[error(transparent)]
        CobRetrieve(#[from] cob::error::Retrieve<PeerRefsError>),
    }

    #[derive(Debug, Error)]
    pub(crate) enum Retrieve {
        #[error(transparent)]
        CobRetrieve(#[from] cob::error::Retrieve<PeerRefsError>),
    }
}

/// A `LiteMonorepo` is a rough approximation to the full monorepo used by librad. The aim is to be
/// able to replicate the ref layout and object database of the full monorepo after creating and
/// replicating collaborative objects from a number of project maintainers. We could use the
/// `Testnet` from `radicle-link-test`, however, this seems like it would take a long time to setup
/// and be fiddly to implement because we would keep having to wait for network interactions to
/// occur after each collaborative object update. We don't really care about the network
/// interactions, all we need to do is make sure that the storage ends up containing the right
/// objects and references. In fact, because access to references is abstracted in the
/// `cob::RefsStorage` trait, we can choose whatever ref layout is easiest to implement.
///
/// In this vein then, when we create the `LiteMonorepo` we create a set of secret keys - one for
/// each peer - and save them. As we import issues from github we assign each github user ID to one of
/// these peers (in a round robin fashion) and save the assignment. Then for each issue we create a
/// change for the initial issue creation and then a change for each comment. We use
/// [`PeerRefsStorage]` to talk to `cob`, which saves refs at
/// `refs/namespaces/<project urn>/refs/remotes/<peer URN>/cob/<typename>/<object ID>` which is
/// essentially the same as the librad implementation.
///
/// The lite monorepo ends up looking like this on disk:
///
/// ```
/// ├── git <- the underlying storage
/// ├── peer_identities <- a JSON file mapping peer IDs to the OID of their identity tree
/// ├── peer_map <- A JSON file mapping github user IDs to peer IDs
/// ├── peers  <- files containing secret keys for each peer ID (given by filename)
/// │   ├── hyb1jukxajb5k1nf8mna4jpz1rdqsazybr3pm6tt5qacr66r64m9un
/// │   ├── hybbnun8qz6znu71yfesn77tnjxggw1bgjc6x71fny9r1kofqykrja
/// |   ...
/// └── project_oid <- The OID of the project identity tree
/// ```
pub struct LiteMonorepo {
    root: PathBuf,
    project: Project,
    peers: Peers,
    repo: git2::Repository,
    peer_assignments: PeerAssignments,
    peer_identities: PeerIdentities,
}

impl LiteMonorepo {
    pub(crate) fn create_or_open<P: AsRef<std::path::Path>>(
        root: P,
    ) -> Result<LiteMonorepo, error::CreateOrOpen> {
        if !std::fs::try_exists(&root)? {
            std::fs::create_dir_all(&root)?;
        }
        let peers = Peers::create_or_read(&root.as_ref().join("peers"))?;
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

        let cob_cache_path = root.as_ref().join("cob_cache");
        if !std::fs::try_exists(&cob_cache_path)? {
            std::fs::create_dir_all(&cob_cache_path)?;
        }

        Ok(LiteMonorepo {
            root: root.as_ref().to_path_buf(),
            peers,
            repo,
            peer_assignments,
            peer_identities,
            project,
        })
    }

    pub(crate) fn import_issue(&mut self, issue: &DownloadedIssue) -> Result<(), error::Import> {
        if let Some(ref author) = issue.author_id {
            let creator_id = self.peer_assignments.assign(author)?;
            let (creator_person, creator_key) = self.peer_identities.get(creator_id).unwrap();
            let init_change = init_issue_change(issue, &creator_person.urn());
            let storage = PeerRefsStorage::new(*creator_id, &self.repo);
            let mut object = cob::create_object(
                &storage,
                &self.repo,
                &(creator_key.clone()).into(),
                &creator_person,
                Either::Right(self.project.clone()),
                cob::NewObjectSpec {
                    history: init_change,
                    message: None,
                    typename: TYPENAME.clone(),
                    schema_json: SCHEMA.clone(),
                },
                Some(self.cache_path()),
            )?;

            for comment in &issue.comments {
                if let Some(commentor) = &comment.author_id {
                    let commentor_id = self.peer_assignments.assign(&commentor)?;
                    let (commentor_person, commentor_key) =
                        self.peer_identities.get(commentor_id).unwrap();
                    let storage = PeerRefsStorage::new(*commentor_id, &self.repo);
                    object = cob::update_object(
                        &storage,
                        &(commentor_key.clone()).into(),
                        &self.repo,
                        &commentor_person,
                        Either::Right(self.project.clone()),
                        cob::UpdateObjectSpec {
                            object_id: *object.id(),
                            typename: TYPENAME.clone(),
                            message: None,
                            changes: add_comment_change(
                                comment,
                                &commentor_person.urn(),
                                object.history(),
                            ),
                        },
                        Some(self.cache_path()),
                    )?;
                }
            }
        }
        Ok(())
    }

    pub(crate) fn list_issues(&self) -> Result<usize, error::List> {
        let some_peer = self.peers.some_peer();
        let storage = PeerRefsStorage::new(*some_peer, &self.repo);
        let objs = cob::retrieve_objects(
            &storage,
            &self.repo,
            Either::Right(self.project.clone()),
            &TYPENAME,
            Some(self.cache_path()),
        )?;
        Ok(objs.len())
    }

    pub(crate) fn retrieve_issue(
        &self,
        object_id: &cob::ObjectId,
        use_cache: bool,
    ) -> Result<Option<serde_json::Value>, error::Retrieve> {
        let some_peer = self.peers.some_peer();
        let storage = PeerRefsStorage::new(*some_peer, &self.repo);
        let cache_path = if use_cache {
            Some(self.cache_path())
        } else {
            None
        };
        if let Some(obj) = cob::retrieve_object(
            &storage,
            &self.repo,
            Either::Right(self.project.clone()),
            &TYPENAME,
            object_id,
            cache_path,
        )? {
            let backend = automerge::Backend::load(obj.history().as_ref().to_vec()).unwrap();
            let mut frontend = automerge::Frontend::new();
            frontend.apply_patch(backend.get_patch().unwrap()).unwrap();
            Ok(Some(frontend.state().to_json()))
        } else {
            Ok(None)
        }
    }

    pub(crate) fn issue_info(
        &self,
        object_id: &cob::ObjectId,
    ) -> Result<Option<cob::ChangeGraphInfo>, error::Retrieve> {
        let some_peer = self.peers.some_peer();
        let storage = PeerRefsStorage::new(*some_peer, &self.repo);
        cob::changegraph_info_for_object(
            &storage,
            &self.repo,
            Either::Right(self.project.clone()),
            &TYPENAME,
            object_id,
        )
        .map_err(error::Retrieve::from)
    }

    fn cache_path(&self) -> std::path::PathBuf {
        self.root.join("cob_cache")
    }
}

impl std::fmt::Debug for LiteMonorepo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "LiteMonorepo{{ {} }}", self.root.display())
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
            d.add_change(LocalChange::set(
                automerge::Path::root().key("github_issue_number"),
                automerge::Value::Primitive(automerge::Primitive::Str(
                    issue.number.to_string().into(),
                )),
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
    let changes: Vec<automerge::Change> = automerge::Change::load_document(hist).unwrap();
    let patch = backend.apply_changes(changes).unwrap();
    frontend.apply_patch(patch).unwrap();

    let (_, change) = frontend
        .change::<_, _, automerge::InvalidChangeRequest>(None, |d| {
            let comments_len = match d.value_at_path(&automerge::Path::root().key("comments")) {
                Some(automerge::Value::List(elems)) => elems.len(),
                _ => panic!("comments must be a list due to the schema"),
            };
            let comment_path = automerge::Path::root()
                .key("comments")
                .index(comments_len as u32);
            let comment_map = automerge::Value::Map(HashMap::new());
            d.add_change(LocalChange::insert(comment_path.clone(), comment_map))?;

            d.add_change(LocalChange::set(
                comment_path.clone().key( "commenter_urn"),
                automerge::Value::Primitive(automerge::Primitive::Str(
                    commentor_urn.to_string().into(),
                )),
            ))?;

            d.add_change(LocalChange::set(
                comment_path.clone().key( "comment"), to_text(comment.body.as_str())
            ))?;

            d.add_change(LocalChange::set(
                comment_path.key("created_at"),
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
