#![feature(async_closure)]

use std::path::PathBuf;

use clap::Clap;

mod downloaded_issue;
mod download;

#[derive(Clap)]
struct Args {
    /// The directory 
    #[clap(short, long, default_value="./data")] 
    data_dir: PathBuf,
    #[clap(subcommand)]
    command: Command,
}

#[derive(Clap)]
enum Command {
    DownloadIssues{
        repo: String,
    }
}

#[tokio::main]
async fn main() {
    let args = Args::parse();
    match args.command {
        Command::DownloadIssues{repo} => {
            let components: Vec<&str> = repo.split("/").collect();
            match &components[..] {
                &[owner, repo] => {
                    let storage = download::Storage::new(args.data_dir);
                    match download::download(owner.to_string(), repo.to_string(), storage).await {
                        Ok(()) => println!("Done"),
                        Err(e) => eprintln!("Failed: {}", e),
                    }
                },
                _ => eprintln!("Invalid repo format"),
            }
        }
    };
}



