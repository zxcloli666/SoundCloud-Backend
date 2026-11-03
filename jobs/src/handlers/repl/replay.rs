use backend_contracts::ReplReplayPayload;
use sqlx::PgPool;
use tracing::{info, warn};

use crate::metrics;
use crate::queue::{JobError, JobResult};

const REPLAY_LIMIT: i64 = 100;

pub(super) async fn run(pool: &PgPool, payload: ReplReplayPayload) -> JobResult {
    let dead = sqlx::query_file!(
        "queries/repl/dead_list.sql",
        payload.node,
        payload.id,
        REPLAY_LIMIT
    )
    .fetch_all(pool)
    .await
    .map_err(JobError::retryable)?;
    let mut failed = Vec::new();
    for command in dead {
        let outcome =
            sqlx::query_file_scalar!("queries/repl/replay_one.sql", command.node, command.id)
                .fetch_one(pool)
                .await
                .map_err(JobError::retryable)?;
        match outcome.as_str() {
            "ok" => {
                metrics::record_repl_replay("ok");
                info!(node = command.node, id = %command.id, "dead command replayed");
            }
            "missing" => metrics::record_repl_replay("missing"),
            _ => {
                metrics::record_repl_replay("failed");
                warn!(node = command.node, id = %command.id, outcome, "dead command failed again");
                failed.push(format!("{}/{}: {outcome}", command.node, command.id));
            }
        }
    }
    if payload.id.is_some() && !failed.is_empty() {
        return Err(JobError::permanent(anyhow::anyhow!(failed.join("; "))));
    }
    Ok(())
}
