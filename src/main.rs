#![feature(async_closure)]
#![feature(path_try_exists)]

use std::path::PathBuf;

use clap::Clap;
use cob::ObjectId;
use indicatif::{ProgressBar, ProgressStyle};

mod download;
mod downloaded_issue;
mod graphql;
mod repo_name;
use repo_name::RepoName;
mod lite_monorepo;
use lite_monorepo::LiteMonorepo;
mod peer_assignments;
mod peer_identities;
mod peer_refs_storage;
mod peers;

#[derive(Debug, Clone, Hash, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
struct GithubUserId(String);

#[derive(Clap)]
struct Args {
    /// The directory
    #[clap(short, long, default_value = "./data")]
    data_dir: PathBuf,
    #[clap(subcommand)]
    command: Command,
}

#[derive(Clap)]
enum Command {
    DownloadIssues {
        #[clap(short, long)]
        token_file: String,
        repo: RepoName,
    },
    ImportIssues {
        repo: RepoName,
    },
    CountImportedIssues {
        repo: RepoName,
    },
    RetrieveIssue {
        repo: RepoName,
        object_id: ObjectId,
        #[clap(long)]
        no_cache: bool,
    },
    IssueChangeGraphInfo {
        repo: RepoName,
        object_id: ObjectId,
        #[clap(long)]
        just_graphviz: bool,
    },
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();
    let args = Args::parse();
    match args.command {
        Command::DownloadIssues { token_file, repo } => {
            let token = std::fs::read_to_string(token_file).unwrap();
            let repo_storage_dir = args
                .data_dir
                .join(repo.owner.as_str())
                .join(repo.name.as_str())
                .join("download");
            if !std::fs::try_exists(&repo_storage_dir).unwrap() {
                std::fs::create_dir_all(&repo_storage_dir).unwrap();
            }
            let storage = download::Storage::new(repo_storage_dir).unwrap();
            let crab = octocrab::OctocrabBuilder::default()
                .personal_token(token.trim().to_string())
                .build()
                .unwrap();
            match download::download(crab, repo, storage).await {
                Ok(()) => println!("Done"),
                Err(e) => eprintln!("Failed: {}", e),
            }
        }
        Command::ImportIssues { repo } => {
            let storage_root = args
                .data_dir
                .join(repo.owner.as_str())
                .join(repo.name.as_str());
            let monorepo_root = storage_root.join("monorepo");
            let mut monorepo = LiteMonorepo::create_or_open(monorepo_root).unwrap();
            let issue_storage_dir = storage_root.join("download");
            let storage = download::Storage::new(issue_storage_dir).unwrap();
            let issues = storage.issues().unwrap();
            let bar = ProgressBar::new(issues.len() as u64);
            bar.set_style(
                ProgressStyle::default_bar()
                    .template("[{elapsed_precise}] {bar:40.yellow/blue} {pos:>7}/{len:7}"),
            );
            for issue in issues.iter() {
                bar.inc(1);
                match monorepo.import_issue(issue) {
                    Ok(()) => {}
                    Err(e) => {
                        eprintln!("Failed to import issue: {:?}", e);
                        return;
                    }
                }
            }
            bar.finish();
        }
        Command::CountImportedIssues { repo } => {
            let storage_root = args
                .data_dir
                .join(repo.owner.as_str())
                .join(repo.name.as_str());
            let monorepo_root = storage_root.join("monorepo");
            let monorepo = LiteMonorepo::create_or_open(monorepo_root).unwrap();
            match monorepo.list_issues() {
                Ok(n) => println!("There are {} issues", n),
                Err(e) => eprintln!("Error retrieving issues {}", e),
            }
        }
        Command::IssueChangeGraphInfo {
            repo,
            object_id,
            just_graphviz,
        } => {
            let storage_root = args
                .data_dir
                .join(repo.owner.as_str())
                .join(repo.name.as_str());
            let monorepo_root = storage_root.join("monorepo");
            let monorepo = LiteMonorepo::create_or_open(monorepo_root).unwrap();
            match monorepo.issue_info(&object_id) {
                Ok(Some(i)) => {
                    if just_graphviz {
                        println!("{}", i.dotviz);
                    } else {
                        println!("Tips of change graph are: {:?}", i.tips);
                        println!("Change graph has {} nodes", i.number_of_nodes);
                    }
                }
                Ok(None) => println!("no such issue"),
                Err(e) => eprintln!("Error retrieving issue {:?}", e),
            }
        }
        Command::RetrieveIssue {
            repo,
            object_id,
            no_cache,
        } => {
            let storage_root = args
                .data_dir
                .join(repo.owner.as_str())
                .join(repo.name.as_str());
            let monorepo_root = storage_root.join("monorepo");
            let monorepo = LiteMonorepo::create_or_open(monorepo_root).unwrap();
            match monorepo.retrieve_issue(&object_id, !no_cache) {
                Ok(Some(json)) => {
                    println!("{}", json);
                }
                Ok(None) => println!("null"),
                Err(e) => eprintln!("Error retrieving issue {}", e),
            }
        }
    };
}
