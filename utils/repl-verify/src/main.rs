use std::env;
use std::process::ExitCode;
use std::time::Duration;

use repl_verify::{Mode, Sweeper, Verifier, probe_once, repair_once, sweep_once};
use sqlx::PgPool;
use tokio_util::sync::CancellationToken;
use tracing_subscriber::EnvFilter;

const USAGE: &str = "usage: repl-verify sweep [--once]
       repl-verify verify [--dry-run] [--once]
DATABASE_URL names the node's database";
const QUIET_POLLS: u32 = 6;
const POLL: Duration = Duration::from_secs(5);

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();
    let args: Vec<String> = env::args().skip(1).collect();
    let once = args.iter().any(|arg| arg == "--once");
    let dry = args.iter().any(|arg| arg == "--dry-run");
    let Ok(url) = env::var("DATABASE_URL") else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    let pool = match PgPool::connect(&url).await {
        Ok(pool) => pool,
        Err(error) => {
            eprintln!("repl-verify: connect: {error}");
            return ExitCode::FAILURE;
        }
    };
    let mode = if dry { Mode::Dry } else { Mode::Repair };
    let result = match (args.first().map(String::as_str), once) {
        (Some("sweep"), true) => sweep_until_empty(&pool).await,
        (Some("verify"), true) => verify_cycle(&pool, dry).await,
        (Some("sweep"), false) => {
            let _ = Sweeper::new(pool).run(stop_on_ctrl_c()).await;
            Ok(())
        }
        (Some("verify"), false) => {
            let _ = Verifier::new(pool, mode).run(stop_on_ctrl_c()).await;
            Ok(())
        }
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("repl-verify: {error}");
            ExitCode::FAILURE
        }
    }
}

fn stop_on_ctrl_c() -> CancellationToken {
    let stop = CancellationToken::new();
    let trigger = stop.clone();
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        trigger.cancel();
    });
    stop
}

async fn sweep_until_empty(pool: &PgPool) -> Result<(), sqlx::Error> {
    loop {
        let sweep = sweep_once(pool, 500).await?;
        println!(
            "{} sent={} skipped={} gone={} queued={}",
            sweep.state, sweep.sent, sweep.skipped, sweep.gone, sweep.queued
        );
        if sweep.state == "legacy" || (sweep.state == "ran" && sweep.work() == 0) {
            return Ok(());
        }
        if sweep.state != "ran" || sweep.sent + sweep.gone == 0 {
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }
}

async fn verify_cycle(pool: &PgPool, dry: bool) -> Result<(), sqlx::Error> {
    sqlx::query_file!("queries/restart.sql")
        .execute(pool)
        .await?;
    let mut probes = 0;
    loop {
        let probe = probe_once(pool, 20, 500).await?;
        if probe.state == "idle" {
            println!("idle: this database is not core in split mode");
            return Ok(());
        }
        probes += probe.probes;
        if probe.wrapped {
            break;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    println!("probed every table: {probes} chunks");
    let mut quiet = 0;
    let mut repairs = 0;
    let mut rechecks = 0;
    while quiet < QUIET_POLLS {
        tokio::time::sleep(POLL).await;
        let rows = repair_once(pool, dry, 1000).await?;
        if rows.is_empty() {
            quiet += 1;
            continue;
        }
        quiet = 0;
        for row in rows {
            if row.recheck() {
                rechecks += 1;
                continue;
            }
            repairs += 1;
            println!(
                "{}\t{}\t{}\t{}\t{}{}",
                row.peer,
                row.rel,
                row.k.as_deref().unwrap_or("-"),
                row.action,
                row.attempt,
                if row.unresolved() { "\tunresolved" } else { "" }
            );
        }
    }
    println!(
        "{repairs} {} after {} quiet seconds, {rechecks} chunks compared key by key",
        if dry { "differences" } else { "repairs" },
        QUIET_POLLS * POLL.as_secs() as u32
    );
    Ok(())
}
