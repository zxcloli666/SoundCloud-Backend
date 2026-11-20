use backend_contracts::JobKind;
use serde::Serialize;
use sqlx::PgPool;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WaitLane {
    Read,
    Queue,
}

impl WaitLane {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Queue => "queue",
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueWait {
    pub lane: WaitLane,
    pub ahead: i64,
    pub eta_seconds: i64,
}

pub async fn load(
    pool: &PgPool,
    kind: JobKind,
    lane: WaitLane,
) -> Result<Option<QueueWait>, sqlx::Error> {
    let row = sqlx::query_file!(
        "queries/background_jobs/queue_wait.sql",
        kind.as_str(),
        lane.as_str()
    )
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|row| QueueWait {
        lane,
        ahead: row.ahead,
        eta_seconds: row.eta_seconds,
    }))
}
