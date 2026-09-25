use backend_contracts::{CatalogCollection, CatalogCollectionPayload, JobKind, Versioned};
use chrono::{DateTime, Utc};
use serde::ser::SerializeStruct;
use serde::{Serialize, Serializer};
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use crate::cache::ListPageResult;
use crate::common::sc_ids::extract_sc_id;
use crate::error::{AppError, AppResult};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CollectionSync {
    pub status: &'static str,
    pub last_completed_at: Option<DateTime<Utc>>,
    pub retry_after_seconds: u64,
}

#[derive(Debug)]
pub struct CollectionPage {
    pub collection: Vec<Value>,
    pub page: i64,
    pub page_size: i64,
    pub has_more: bool,
    pub sync: CollectionSync,
}

impl Serialize for CollectionPage {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut wire = serializer.serialize_struct("CollectionPage", 7)?;
        wire.serialize_field("collection", &self.collection)?;
        wire.serialize_field("page", &self.page)?;
        wire.serialize_field("pageSize", &self.page_size)?;
        wire.serialize_field("page_size", &self.page_size)?;
        wire.serialize_field("hasMore", &self.has_more)?;
        wire.serialize_field("has_more", &self.has_more)?;
        wire.serialize_field("sync", &self.sync)?;
        wire.end()
    }
}

impl CollectionPage {
    pub fn new(page: ListPageResult<Value>, sync: CollectionSync) -> Self {
        Self {
            collection: page.collection,
            page: page.page,
            page_size: page.page_size,
            has_more: page.has_more,
            sync,
        }
    }

    pub fn empty(sync: CollectionSync, page: i64, limit: i64) -> Self {
        Self {
            collection: Vec::new(),
            page: page.clamp(0, 100),
            page_size: limit,
            has_more: false,
            sync,
        }
    }
}

pub(super) async fn ensure(
    pool: &PgPool,
    collection: CatalogCollection,
    subject_urn: &str,
    owner: bool,
    ttl: u64,
) -> AppResult<CollectionSync> {
    let payload = CatalogCollectionPayload {
        collection,
        subject_id: extract_sc_id(subject_urn).to_owned(),
        owner,
    };
    if !payload.is_valid() {
        return Err(AppError::bad_request("Invalid collection subject"));
    }
    let synced_at = sqlx::query_file_scalar!(
        "queries/cold_refresh/collection_status.sql",
        &payload.subject_id,
        collection.as_str(),
        payload.scope()
    )
    .fetch_optional(pool)
    .await?
    .flatten();
    let fresh = synced_at.is_some_and(|at| {
        let age = Utc::now().signed_duration_since(at).num_seconds();
        age >= 0 && age as u64 <= ttl
    });
    let key = payload.dedup_key();
    let mut retry_after_seconds = 0;
    if !fresh {
        let body = serde_json::to_value(Versioned::V1(payload))
            .map_err(|error| AppError::internal(error.to_string()))?;
        let kind = JobKind::CatalogCollection;
        sqlx::query_file!(
            "queries/cold_refresh/enqueue_entity.sql",
            Uuid::now_v7(),
            kind.as_str(),
            kind.lane().as_str(),
            &key,
            body
        )
        .execute(pool)
        .await?;
        retry_after_seconds =
            sqlx::query_file_scalar!("queries/cold_refresh/collection_retry_after.sql", &key)
                .fetch_optional(pool)
                .await?
                .unwrap_or(5)
                .clamp(5, 1800) as u64;
    }
    Ok(CollectionSync {
        status: if fresh { "ready" } else { "refreshing" },
        last_completed_at: synced_at,
        retry_after_seconds,
    })
}
