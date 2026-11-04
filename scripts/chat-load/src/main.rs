mod admin;
mod api;
mod b64;
mod cli;
mod content;
mod device;
mod fake_pro;
mod layout;
mod pro;
mod prom;
mod report;
mod route;
mod runner;
mod schedule;
mod stats;
mod transport;
mod watch;
mod wire;

use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::time::Duration;

use clap::Parser;

use crate::admin::Admin;
use crate::cli::{Cli, Command, SeedArgs};

pub const FAILED_EXIT: i32 = 3;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    match Cli::parse().command {
        Command::Seed(args) => seed(args).await,
        Command::Purge(args) => {
            let admin = Admin::new(&args.admin.admin, &args.admin.token_file)?;
            admin.purge(Duration::from_secs(args.wait_secs)).await?;
            Ok(())
        }
        Command::Run(args) => {
            if !runner::run(*args).await? {
                std::process::exit(FAILED_EXIT);
            }
            Ok(())
        }
        Command::FakePro(args) => fake_pro::serve(args.bind).await,
    }
}

async fn seed(args: SeedArgs) -> anyhow::Result<()> {
    let admin = Admin::new(&args.admin.admin, &args.admin.token_file)?;
    let seeded = admin
        .seed(args.users, args.devices_per_user, args.groups)
        .await?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&args.out)?;
    file.write_all(&serde_json::to_vec(&seeded)?)?;
    println!(
        "seeded from lt:{}: {} devices in {} chats, layout in {}",
        seeded["first"],
        seeded["devices"].as_array().map_or(0, Vec::len),
        seeded["conversations"].as_array().map_or(0, Vec::len),
        args.out.display()
    );
    Ok(())
}
