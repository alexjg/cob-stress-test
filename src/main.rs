#![feature(async_closure)]
#![feature(path_try_exists)]

use std::path::PathBuf;

use clap::Clap;

mod download;
mod downloaded_issue;
mod repo_name;
use repo_name::RepoName;
mod lite_monorepo;
use lite_monorepo::LiteMonorepo;
mod peer_refs_storage;
mod peer_assignments;
mod peer_identities;
mod peers;

#[derive(Clone, Hash, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
struct GithubUserId(u64);


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
        repo: RepoName 
    },
    CreateMonorepo { repo: RepoName },
    ImportIssues { repo: RepoName },
    ListImportedIssues { repo: RepoName },
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();
    let args = Args::parse();
    match args.command {
        Command::DownloadIssues { token_file, repo } => {
            let token = std::fs::read_to_string(token_file).unwrap();
            let repo_storage_dir = args.data_dir.join(repo.owner.as_str()).join(repo.name.as_str()).join("download");
            if !std::fs::try_exists(&repo_storage_dir).unwrap() {
                std::fs::create_dir_all(&repo_storage_dir).unwrap();
            }
            let storage = download::Storage::new(repo_storage_dir);
            let crab = octocrab::OctocrabBuilder::default()
                .personal_token(token.trim().to_string())
                .build()
                .unwrap();
            match download::download(crab, repo, storage)
                .await
            {
                Ok(()) => println!("Done"),
                Err(e) => eprintln!("Failed: {}", e),
            }
        },
        Command::ImportIssues { repo } => {
            let storage_root = args.data_dir.join(repo.owner.as_str()).join(repo.name.as_str());
            let monorepo_root = storage_root.join("monorepo");
            let mut monorepo = LiteMonorepo::from_root(monorepo_root).unwrap();
            let issue_storage_dir = storage_root.join("download");
            let storage = download::Storage::new(issue_storage_dir);
            let issues = storage.issues().unwrap();
            for issue in issues {
                match monorepo.import_issue(&issue) {
                    Ok(()) => {},
                    Err(e) => {
                        eprintln!("Failed to import issue: {}", e);
                        return
                    }
                }
            }
            ()
        }
        Command::CreateMonorepo { repo } => {
            let storage_root = args.data_dir.join(repo.owner.as_str()).join(repo.name.as_str());
            let monorepo_root = storage_root.join("monorepo");
            let _monorepo = LiteMonorepo::from_root(monorepo_root).unwrap();
        }
        Command::ListImportedIssues { repo } => {
            let storage_root = args.data_dir.join(repo.owner.as_str()).join(repo.name.as_str());
            let monorepo_root = storage_root.join("monorepo");
            let monorepo = LiteMonorepo::from_root(monorepo_root).unwrap();
            match monorepo.list_issues() {
                Ok(n) => println!("There are {} issues", n),
                Err(e) => eprintln!("Error retrieving issues {}", e),
            }
        }
    };
}
