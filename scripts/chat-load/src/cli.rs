use std::net::SocketAddr;
use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

use crate::route::{DEFAULT_MIX, DEFAULT_RELAY_CAP};

#[derive(Parser)]
#[command(
    name = "chat-load",
    about = "Synthetic chat devices that seal, sign, park and send like the desktop"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    Seed(SeedArgs),
    Purge(PurgeArgs),
    Run(Box<RunArgs>),
    FakePro(FakeProArgs),
}

#[derive(Args)]
pub struct AdminArgs {
    #[arg(long)]
    pub admin: String,
    #[arg(long)]
    pub token_file: PathBuf,
}

#[derive(Args)]
pub struct SeedArgs {
    #[command(flatten)]
    pub admin: AdminArgs,
    #[arg(long)]
    pub users: u32,
    #[arg(long, default_value_t = 1)]
    pub devices_per_user: u32,
    #[arg(long)]
    pub groups: u32,
    #[arg(long)]
    pub out: PathBuf,
}

#[derive(Args)]
pub struct PurgeArgs {
    #[command(flatten)]
    pub admin: AdminArgs,
    #[arg(long, default_value_t = 900)]
    pub wait_secs: u64,
}

#[derive(Args)]
pub struct FakeProArgs {
    #[arg(long, default_value = "127.0.0.1:8080")]
    pub bind: SocketAddr,
}

#[derive(Args, Clone)]
pub struct RunArgs {
    #[arg(long = "layout", required = true)]
    pub layouts: Vec<PathBuf>,
    #[arg(long, default_value_t = 1)]
    pub ock_version: u8,
    #[arg(long)]
    pub ock_pub: Option<String>,
    #[arg(long)]
    pub direct: Option<String>,
    #[arg(long = "relay")]
    pub relays: Vec<String>,
    #[arg(long = "pro")]
    pub pros: Vec<String>,
    #[arg(long)]
    pub origin: Option<String>,
    #[arg(long, default_value = DEFAULT_MIX)]
    pub mix: String,
    #[arg(long, default_value_t = DEFAULT_RELAY_CAP)]
    pub relay_cap: usize,
    #[arg(long, value_delimiter = ',', default_values_t = [2_000, 5_000, 10_000, 15_000, 20_000])]
    pub steps: Vec<usize>,
    #[arg(long, default_value_t = 240)]
    pub step_secs: u64,
    #[arg(long, default_value_t = 60)]
    pub spread_secs: u64,
    #[arg(long, default_value_t = 900)]
    pub hold_secs: u64,
    #[arg(long)]
    pub restart_cmd: Option<String>,
    #[arg(long, default_value_t = 600)]
    pub restart_at_secs: u64,
    #[arg(long, default_value_t = 300)]
    pub worst_secs: u64,
    #[arg(long, default_value_t = 0.3)]
    pub parked_share: f64,
    #[arg(long, default_value_t = 0.5)]
    pub focus_share: f64,
    #[arg(long, default_value_t = 20_000)]
    pub hold_ms: u64,
    #[arg(long, default_value_t = 60)]
    pub short_every_secs: u64,
    #[arg(long, default_value_t = 300)]
    pub send_every_secs: u64,
    #[arg(long, default_value_t = 120)]
    pub type_every_secs: u64,
    #[arg(long = "watch")]
    pub watches: Vec<String>,
    #[arg(long, default_value_t = 15)]
    pub watch_every_secs: u64,
    #[arg(long, default_value_t = 0.01)]
    pub abort_error_rate: f64,
    #[arg(long)]
    pub chat_metrics_cmd: Option<String>,
    #[arg(long, default_value_t = 200.0)]
    pub abort_pool_wait_ms: f64,
    #[arg(long)]
    pub music_metrics_cmd: Option<String>,
    #[arg(long, default_value_t = 0.2)]
    pub abort_music_rise: f64,
    #[arg(long, default_value_t = 60)]
    pub baseline_secs: u64,
    #[arg(long, default_value = "chat-load-report.json")]
    pub report: PathBuf,
    #[arg(long, default_value_t = 10)]
    pub status_every_secs: u64,
}
