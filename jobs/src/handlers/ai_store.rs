use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use tracing::debug;

const MODEL_ANSWER_TTL_SECONDS: i64 = 30 * 24 * 60 * 60;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RpcSource {
    Deterministic,
    Llm,
}

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

    pub async fn remember<T: Serialize>(&self, key: &str, answer: &T) {
        let Ok(payload) = serde_json::to_value(answer) else {
            return;
        };
        let stored = sqlx::query_file!(
            "queries/enrich/ai/cache_set.sql",
            key,
            payload,
            MODEL_ANSWER_TTL_SECONDS
        )
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

    pub async fn refund_budget(&self) {
        if self.daily_budget == 0 {
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

    #[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
    async fn a_refunded_unit_can_be_taken_again(pool: PgPool) {
        let store = AiStore::new(pool, 1);

        assert!(store.take_budget().await);
        store.refund_budget().await;
        assert!(store.take_budget().await);
        assert!(!store.take_budget().await);
    }

    #[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
    async fn a_remembered_answer_reads_back_for_a_month(pool: PgPool) -> anyhow::Result<()> {
        let store = AiStore::new(pool.clone(), 1);

        store
            .remember("match:key", &serde_json::json!({"match_id": 2}))
            .await;

        let answer: Option<serde_json::Value> = store.cached("match:key").await;
        assert_eq!(answer, Some(serde_json::json!({"match_id": 2})));
        let lives_a_month: bool = sqlx::query_scalar(
            "SELECT expires_at > now() + interval '29 days' FROM ai_resolver_cache WHERE cache_key = $1",
        )
        .bind("match:key")
        .fetch_one(&pool)
        .await?;
        assert!(lives_a_month);
        Ok(())
    }
}
