use backend_contracts::{
    CatalogCollection, CatalogCollectionPayload, CatalogEntity, CollectionFailure, JobKind,
    LIBRARY_READ_BOOST, OWNER_COLLECTION_PRIORITY, STAR_PRIORITY_BONUS,
};
use chrono::{DateTime, Utc};
use serde::ser::SerializeStruct;
use serde::{Serialize, Serializer};
use serde_json::Value;
use sqlx::PgPool;
use wt::EnqueuedJob;

use crate::background_jobs::queue;
use crate::background_jobs::wait::{self, QueueWait, WaitLane};
use crate::cache::ListPageResult;
use crate::common::pagination::last_page;
use crate::common::sc_ids::{extract_sc_id, user_id_variants};
use crate::error::{AppError, AppResult};

use super::VIEWER_PRIORITY;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CollectionSync {
    pub status: &'static str,
    pub last_completed_at: Option<DateTime<Utc>>,
    pub queued_since: Option<DateTime<Utc>>,
    pub error_code: Option<&'static str>,
    pub retry_after_seconds: u64,
    pub wait: Option<QueueWait>,
    pub synced_items: Option<i64>,
}

struct Pass {
    synced_at: Option<DateTime<Utc>>,
    complete: Option<bool>,
    page_count: Option<i64>,
    item_count: Option<i64>,
    queued: bool,
}

impl Pass {
    fn reads_first_page(&self) -> bool {
        !self.queued || self.complete.unwrap_or(true) || self.page_count.unwrap_or(0) == 0
    }

    fn synced_items(&self) -> Option<i64> {
        match self.complete {
            Some(false) => self.item_count,
            _ => None,
        }
    }
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
            page: page.clamp(0, last_page(limit)),
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
    let key = payload.dedup_key();
    let pass = load_pass(pool, &payload, &key).await?;
    let synced_at = pass.synced_at;
    let fresh = synced_at.is_some_and(|at| {
        let age = Utc::now().signed_duration_since(at).num_seconds();
        age >= 0 && age as u64 <= ttl
    });
    if fresh {
        return Ok(CollectionSync {
            status: "ready",
            last_completed_at: synced_at,
            queued_since: None,
            error_code: None,
            retry_after_seconds: 0,
            wait: None,
            synced_items: None,
        });
    }
    let failure = sqlx::query_file!(
        "queries/cold_refresh/collection_failure.sql",
        &key,
        synced_at,
        &user_id_variants(&payload.subject_id)
    )
    .fetch_optional(pool)
    .await?;
    let held = failure
        .as_ref()
        .is_some_and(|failure| failure.hold_seconds != Some(0));
    let first_page = pass.reads_first_page();
    if !held {
        let priority = if owner {
            read_priority(OWNER_COLLECTION_PRIORITY, first_page)
        } else {
            VIEWER_PRIORITY
        };
        enqueue_pass(pool, payload, priority).await?;
    }
    let job = sqlx::query_file!("queries/cold_refresh/collection_job.sql", &key)
        .fetch_optional(pool)
        .await?;
    let retry_after_seconds = match &failure {
        Some(failure) if held => failure.hold_seconds.unwrap_or(0),
        _ => job.as_ref().map_or(5, |job| job.retry_after_seconds),
    };
    let wait = if owner && !held {
        let lane = if first_page {
            WaitLane::Read
        } else {
            WaitLane::Queue
        };
        wait::load(pool, JobKind::CatalogCollection, lane).await?
    } else {
        None
    };
    Ok(CollectionSync {
        status: "refreshing",
        last_completed_at: synced_at,
        queued_since: job.map(|job| job.queued_since),
        error_code: failure
            .and_then(|failure| CollectionFailure::from_last_error(&failure.code))
            .map(CollectionFailure::as_str),
        retry_after_seconds: retry_after_seconds.max(0) as u64,
        wait,
        synced_items: pass.synced_items(),
    })
}

pub(super) async fn sync_now(
    pool: &PgPool,
    collection: CatalogCollection,
    sc_user_id: &str,
    priority: i16,
) -> AppResult<()> {
    let payload = CatalogCollectionPayload {
        collection,
        subject_id: extract_sc_id(sc_user_id).to_owned(),
        owner: true,
    };
    if !payload.is_valid() {
        return Err(AppError::bad_request("Invalid collection subject"));
    }
    let key = payload.dedup_key();
    let pass = load_pass(pool, &payload, &key).await?;
    let failure = sqlx::query_file!(
        "queries/cold_refresh/collection_failure.sql",
        &key,
        pass.synced_at,
        &user_id_variants(&payload.subject_id)
    )
    .fetch_optional(pool)
    .await?;
    if failure.is_some_and(|failure| failure.hold_seconds != Some(0)) {
        return Ok(());
    }
    let priority = read_priority(priority, pass.reads_first_page());
    enqueue_pass(pool, payload, priority).await
}

fn read_priority(priority: i16, first_page: bool) -> i16 {
    if first_page {
        priority + LIBRARY_READ_BOOST
    } else {
        priority
    }
}

async fn load_pass(
    pool: &PgPool,
    payload: &CatalogCollectionPayload,
    key: &str,
) -> AppResult<Pass> {
    Ok(sqlx::query_file_as!(
        Pass,
        "queries/cold_refresh/collection_status.sql",
        &payload.subject_id,
        payload.collection.as_str(),
        payload.scope(),
        key
    )
    .fetch_one(pool)
    .await?)
}

async fn enqueue_pass(
    pool: &PgPool,
    payload: CatalogCollectionPayload,
    priority: i16,
) -> AppResult<()> {
    let star_candidates = if payload.collection.subject() == CatalogEntity::User {
        vec![payload.subject_id.clone()]
    } else {
        Vec::new()
    };
    let job = queue::job(
        JobKind::CatalogCollection,
        payload.dedup_key(),
        payload,
        priority,
    )?;
    queue::enqueue(
        pool,
        vec![EnqueuedJob {
            raise_priority: true,
            star_candidates,
            star_bonus: STAR_PRIORITY_BONUS,
            ..job
        }],
        None,
    )
    .await?;
    Ok(())
}
