use backend_contracts::pipeline::RpcSource;
use serde::Serialize;
use serde::de::DeserializeOwned;
use sqlx::PgPool;
use tracing::debug;

pub struct AiStore {
    pool: PgPool,
    daily_budget: u64,
}

impl AiStore {
    pub fn new(pool: PgPool, daily_budget: u64) -> Self {
        Self { pool, daily_budget }
    }

    pub async fn cached<T: DeserializeOwned>(&self, key: &str) -> Option<T> {
        let stored = sqlx::query_file_scalar!("queries/enrich/ai/cache_get.sql", key)
            .fetch_optional(&self.pool)
            .await
            .ok()??;
        serde_json::from_value(stored).ok()
    }

    pub async fn remember<T: Serialize>(&self, key: &str, reply: &T, ttl_seconds: i64) {
        let Ok(payload) = serde_json::to_value(reply) else {
            return;
        };
        let stored =
            sqlx::query_file!("queries/enrich/ai/cache_set.sql", key, payload, ttl_seconds)
                .execute(&self.pool)
                .await;
        if let Err(error) = stored {
            debug!(%error, "ai cache write failed");
        }
    }

    pub async fn take_budget(&self) -> bool {
        if self.daily_budget == 0 {
            return true;
        }
        match sqlx::query_file_scalar!("queries/enrich/ai/budget_take.sql")
            .fetch_one(&self.pool)
            .await
        {
            Ok(spent) => spent as u64 <= self.daily_budget,
            Err(error) => {
                debug!(%error, "ai budget accounting failed, allowing request");
                true
            }
        }
    }

    pub async fn settle_budget(&self, source: RpcSource) {
        if self.daily_budget == 0 || source == RpcSource::Llm {
            return;
        }
        let refunded = sqlx::query_file!("queries/enrich/ai/budget_refund.sql")
            .execute(&self.pool)
            .await;
        if let Err(error) = refunded {
            debug!(%error, "ai budget refund failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[sqlx::test(migrations = "../api/migrations")]
    async fn only_a_language_model_answer_keeps_its_budget_unit(pool: PgPool) {
        let store = AiStore::new(pool, 1);

        assert!(store.take_budget().await);
        store.settle_budget(RpcSource::Deterministic).await;
        assert!(store.take_budget().await);
        store.settle_budget(RpcSource::Llm).await;
        assert!(!store.take_budget().await);
    }
}
