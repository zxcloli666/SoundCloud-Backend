use std::sync::Arc;
use std::time::Duration;

use sqlx::PgPool;
use tracing::warn;

use sc_transport::{EgressFuture, EgressHealthStore, EgressState};

pub const EGRESS_APP: &str = "jobs";

pub struct PgEgressHealth {
    pg: PgPool,
}

impl PgEgressHealth {
    pub fn new(pg: PgPool) -> Arc<Self> {
        Arc::new(Self { pg })
    }
}

impl EgressHealthStore for PgEgressHealth {
    fn publish_open<'a>(
        &'a self,
        channel: &'a str,
        app: &'a str,
        cooldown: Duration,
    ) -> EgressFuture<'a, ()> {
        Box::pin(async move {
            let seconds = cooldown.as_secs_f64();
            if let Err(error) =
                sqlx::query_file!("queries/sc/publish_egress_open.sql", channel, app, seconds)
                    .execute(&self.pg)
                    .await
            {
                warn!(%error, channel, "failed to publish an open egress breaker");
            }
        })
    }

    fn publish_closed<'a>(&'a self, channel: &'a str) -> EgressFuture<'a, ()> {
        Box::pin(async move {
            if let Err(error) = sqlx::query_file!("queries/sc/publish_egress_closed.sql", channel)
                .execute(&self.pg)
                .await
            {
                warn!(%error, channel, "failed to withdraw an egress breaker");
            }
        })
    }

    fn remaining<'a>(&'a self, channel: &'a str) -> EgressFuture<'a, EgressState> {
        Box::pin(async move {
            match sqlx::query_file_scalar!("queries/sc/egress_remaining.sql", channel)
                .fetch_optional(&self.pg)
                .await
            {
                Ok(Some(seconds)) if seconds > 0.0 => {
                    EgressState::Open(Duration::from_secs_f64(seconds))
                }
                Ok(_) => EgressState::Closed,
                Err(error) => {
                    warn!(%error, channel, "failed to read the shared egress breaker");
                    EgressState::Unknown
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sc_transport::EGRESS_RELAY_LUA;

    async fn schema(pool: &PgPool) -> anyhow::Result<()> {
        sqlx::query(include_str!(
            "../../../api/migrations/0109_sc_egress_health.sql"
        ))
        .execute(pool)
        .await?;
        Ok(())
    }

    #[sqlx::test(migrations = false)]
    async fn the_background_queries_round_trip_against_the_shared_table(
        pool: PgPool,
    ) -> anyhow::Result<()> {
        schema(&pool).await?;
        let store = PgEgressHealth::new(pool.clone());

        assert_eq!(
            store.remaining(EGRESS_RELAY_LUA).await,
            EgressState::Closed,
            "an unknown channel must read as healthy, not as unreadable"
        );

        store
            .publish_open(EGRESS_RELAY_LUA, EGRESS_APP, Duration::from_secs(60))
            .await;
        match store.remaining(EGRESS_RELAY_LUA).await {
            EgressState::Open(left) => assert!(
                left > Duration::from_secs(50) && left <= Duration::from_secs(60),
                "the cooldown must come back from the database clock, saw {left:?}"
            ),
            other => panic!("the published outage must read back as open, saw {other:?}"),
        }

        let opened_by: String =
            sqlx::query_scalar("SELECT opened_by FROM sc_egress_health WHERE channel = $1")
                .bind(EGRESS_RELAY_LUA)
                .fetch_one(&pool)
                .await?;
        assert_eq!(opened_by, EGRESS_APP);

        store.publish_closed(EGRESS_RELAY_LUA).await;
        assert_eq!(store.remaining(EGRESS_RELAY_LUA).await, EgressState::Closed);
        Ok(())
    }
}
