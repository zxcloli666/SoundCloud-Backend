use std::sync::Arc;
use std::time::Duration;

use sqlx::PgPool;
use tracing::warn;

use sc_transport::{EgressFuture, EgressHealthStore, EgressState};

pub const EGRESS_APP: &str = "api";

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
    use sc_transport::{EGRESS_RELAY_LUA, EGRESS_RELAY_RAW, EgressHealth, RelayRead};
    use sqlx::PgPool;

    fn process(app: &'static str, channel: &'static str, pg: &PgPool) -> EgressHealth {
        EgressHealth::new(channel, app, Some(PgEgressHealth::new(pg.clone())))
    }

    async fn trip(health: &EgressHealth) {
        for _ in 0..4 {
            health.observe(&RelayRead::<()>::Unavailable).await;
        }
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn an_outage_one_process_hit_reaches_the_other_and_stays_on_its_own_egress(pg: PgPool) {
        let serving = process("api", EGRESS_RELAY_LUA, &pg);
        let background = process("jobs", EGRESS_RELAY_LUA, &pg);
        let background_raw = process("jobs", EGRESS_RELAY_RAW, &pg);

        trip(&serving).await;

        assert!(
            background.is_open().await,
            "a channel one process found dead must be closed for the other too"
        );
        assert!(
            !background_raw.is_open().await,
            "a dead lua channel must not close the raw apiv2 channel"
        );

        let opened_by: String =
            sqlx::query_scalar("SELECT opened_by FROM sc_egress_health WHERE channel = $1")
                .bind(EGRESS_RELAY_LUA)
                .fetch_one(&pg)
                .await
                .expect("the trip is recorded");
        assert_eq!(opened_by, "api");

        let left = PgEgressHealth::new(pg.clone())
            .remaining(EGRESS_RELAY_LUA)
            .await;
        match left {
            EgressState::Open(left) => assert!(
                left > Duration::from_secs(50) && left <= Duration::from_secs(60),
                "the remaining cooldown must come from the database clock, saw {left:?}"
            ),
            other => panic!("the shared state must report an open channel, saw {other:?}"),
        }
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_recovery_in_one_process_frees_the_other_before_the_cooldown_lapses(pg: PgPool) {
        let serving = process("api", EGRESS_RELAY_LUA, &pg);
        let background = process("jobs", EGRESS_RELAY_LUA, &pg);

        trip(&serving).await;
        assert!(background.is_open().await);

        serving.observe(&RelayRead::<()>::Found(())).await;
        assert!(
            !serving.is_open().await,
            "the process that saw the channel answer must reopen it for itself at once"
        );

        tokio::time::sleep(Duration::from_millis(1100)).await;
        assert!(
            !background.is_open().await,
            "a withdrawn outage must free the other process without waiting out the cooldown"
        );
    }
}
