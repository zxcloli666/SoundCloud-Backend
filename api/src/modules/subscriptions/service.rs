use std::sync::Arc;

use serde::Serialize;
use sqlx::PgPool;

use crate::error::AppResult;

#[derive(Debug, Clone, Serialize)]
pub struct Subscription {
    pub user_urn: String,
    pub exp_date: i64,
}

pub struct SubscriptionsService {
    pg: PgPool,
    always_premium: bool,
}

impl SubscriptionsService {
    pub fn new(pg: PgPool, always_premium: bool) -> Arc<Self> {
        Arc::new(Self { pg, always_premium })
    }

    pub async fn is_premium(&self, user_urn: &str) -> AppResult<bool> {
        if self.always_premium {
            return Ok(true);
        }
        let now = chrono::Utc::now().timestamp();
        let variants = crate::common::sc_ids::user_id_variants(user_urn);
        let row =
            sqlx::query_file_scalar!("queries/subscriptions/service/get_exp_date.sql", &variants)
                .fetch_optional(&self.pg)
                .await?;
        Ok(row.is_some_and(|exp| exp > now))
    }

    pub async fn list(&self) -> AppResult<Vec<Subscription>> {
        let rows = sqlx::query_file!("queries/subscriptions/service/list_all.sql")
            .fetch_all(&self.pg)
            .await?;
        Ok(rows
            .into_iter()
            .map(|r| Subscription {
                user_urn: r.user_urn,
                exp_date: r.exp_date,
            })
            .collect())
    }

    pub async fn upsert(&self, user_urn: &str, exp_date: i64) -> AppResult<()> {
        sqlx::query_file!(
            "queries/subscriptions/service/upsert.sql",
            crate::common::sc_ids::extract_sc_id(user_urn),
            exp_date
        )
        .execute(&self.pg)
        .await?;
        Ok(())
    }

    pub async fn remove(&self, user_urn: &str) -> AppResult<u64> {
        let variants = crate::common::sc_ids::user_id_variants(user_urn);
        let result = sqlx::query_file!("queries/subscriptions/service/remove.sql", &variants)
            .execute(&self.pg)
            .await?;
        Ok(result.rows_affected())
    }
}
