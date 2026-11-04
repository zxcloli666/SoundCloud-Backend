use std::sync::Arc;
use std::time::Duration;

use backend_contracts::CatalogEntity;
use chrono::Utc;
use mini_moka::sync::Cache;
use sqlx::PgPool;
use uuid::Uuid;

use crate::common::sc_ids::extract_sc_id;
use crate::modules::cold_refresh::VIEWER_PRIORITY;
use crate::modules::cold_refresh::entity::enqueue_entity;
use crate::modules::resolve::{Adopted, CatalogMiss};
use crate::modules::users::UserRow;

const FRESH_FOR: Duration = Duration::from_secs(10 * 60);
const CHECKED_CAPACITY: u64 = 65_536;
const WAIT_FOR_SOUNDCLOUD: Duration = Duration::from_secs(2);

pub struct PublicUsers {
    pg: PgPool,
    miss: Arc<CatalogMiss>,
    checked: Cache<String, ()>,
}

impl PublicUsers {
    pub fn new(pg: PgPool, miss: Arc<CatalogMiss>) -> Arc<Self> {
        Arc::new(Self {
            pg,
            miss,
            checked: Cache::builder()
                .max_capacity(CHECKED_CAPACITY)
                .time_to_live(FRESH_FOR)
                .build(),
        })
    }

    pub async fn refresh(&self, session: Uuid, reader: &str, row: &UserRow) -> bool {
        let id = extract_sc_id(&row.sc_user_id).to_owned();
        if is_fresh(row) || self.checked.get(&id).is_some() {
            return false;
        }
        self.checked.insert(id.clone(), ());
        let read = tokio::time::timeout(WAIT_FOR_SOUNDCLOUD, self.miss.user(session, &id)).await;
        if matches!(read, Ok(Adopted::Stored)) {
            return true;
        }
        if let Err(error) = enqueue_entity(
            &self.pg,
            CatalogEntity::User,
            &id,
            None,
            Some(reader),
            VIEWER_PRIORITY,
        )
        .await
        {
            tracing::debug!(%error, "public user refresh enqueue deferred");
        }
        false
    }
}

fn is_fresh(row: &UserRow) -> bool {
    let age = Utc::now().signed_duration_since(row.sc_synced_at);
    age.to_std().is_ok_and(|age| age < FRESH_FOR)
}

#[cfg(test)]
#[path = "public_tests.rs"]
mod tests;
