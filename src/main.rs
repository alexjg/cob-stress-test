#![feature(async_closure)]
#![feature(path_try_exists)]

use std::path::PathBuf;

use clap::Clap;

mod download;
mod downloaded_issue;

#[derive(Clap)]
struct Args {
    /// The directory
    #[clap(short, long, default_value = "./data")]
    data_dir: PathBuf,
    #[clap(short, long)]
    token_file: String,
    #[clap(subcommand)]
    command: Command,
}

#[derive(Clap)]
enum Command {
    DownloadIssues { repo: String },
}

#[tokio::main]
async fn main() {
    let args = Args::parse();
    match args.command {
        Command::DownloadIssues { repo } => {
            let components: Vec<&str> = repo.split("/").collect();
            match &components[..] {
                &[owner, repo] => {
                    let token = std::fs::read_to_string(args.token_file).unwrap();
                    let repo_storage_dir = args.data_dir.join(owner).join(repo);
                    if !std::fs::try_exists(&repo_storage_dir).unwrap() {
                        std::fs::create_dir_all(&repo_storage_dir).unwrap();
                    }
                    let storage = download::Storage::new(repo_storage_dir);
                    let crab = octocrab::OctocrabBuilder::default()
                        .personal_token(token.trim().to_string())
                        .build()
                        .unwrap();
                    match download::download(crab, owner.to_string(), repo.to_string(), storage)
                        .await
                    {
                        Ok(()) => println!("Done"),
                        Err(e) => eprintln!("Failed: {}", e),
                    }
                }
                _ => eprintln!("Invalid repo format"),
            }
        }
    };
}
