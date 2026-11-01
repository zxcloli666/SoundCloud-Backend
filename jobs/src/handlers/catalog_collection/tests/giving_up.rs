use std::sync::Arc;

use backend_contracts::{CatalogCollection, CollectionFailure};
use chrono::{Duration, Utc};
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;
use wreq::StatusCode;

use super::super::{CatalogCollectionHandler, give_up_or_wait, state};
use super::setup;
use crate::handlers::catalog_read::PublicCatalogReader;
use crate::handlers::catalog_remote::{CatalogRemote, owner_error, postpone, public_error};
use crate::handlers::playlist_observe::PlaylistReadError;
use crate::queue::JobError;

fn started(ago: Duration) -> state::Snapshot {
    state::Snapshot {
        snapshot_id: Uuid::now_v7(),
        started_at: Utc::now() - ago,
        next_cursor: None,
        page_count: 0,
        item_count: 0,
        complete: false,
    }
}

fn past_the_first_page(ago: Duration) -> state::Snapshot {
    state::Snapshot {
        next_cursor: Some("https://api-v2.soundcloud.com/users/42/likes?offset=2".to_owned()),
        page_count: 3,
        item_count: 600,
        ..started(ago)
    }
}

fn public_refusal(status: u16) -> JobError {
    public_error(sc_transport::ScError::Api {
        status,
        body: Value::Null,
        retry_after_sec: Some(60),
    })
}

fn owner_refusal(status: StatusCode) -> JobError {
    owner_error(PlaylistReadError::Api {
        status,
        body: Value::Null,
        retry_after_seconds: Some(60),
    })
}

fn failure_of(error: &JobError) -> Option<CollectionFailure> {
    match error {
        JobError::Permanent(error) => CollectionFailure::from_last_error(&error.to_string()),
        _ => None,
    }
}

fn delay_of(error: &JobError) -> Option<u64> {
    match error {
        JobError::Postponed { delay, .. } => Some(delay.as_secs()),
        _ => None,
    }
}

#[test]
fn a_missing_collection_gives_up_on_the_second_404() {
    for missing in [public_refusal(404), owner_refusal(StatusCode::NOT_FOUND)] {
        let given_up = give_up_or_wait(&started(Duration::minutes(30)), missing);
        assert_eq!(failure_of(&given_up), Some(CollectionFailure::NotFound));
        assert!(given_up.to_string().starts_with("not_found: "));
    }
    for missing in [public_refusal(404), owner_refusal(StatusCode::NOT_FOUND)] {
        let waiting = give_up_or_wait(&started(Duration::minutes(5)), missing);
        assert_eq!(delay_of(&waiting), Some(1800));
    }
}

#[test]
fn a_forbidden_collection_gives_up_after_six_hours() {
    for refused in [public_refusal(403), owner_refusal(StatusCode::FORBIDDEN)] {
        let given_up = give_up_or_wait(&started(Duration::hours(7)), refused);
        assert_eq!(failure_of(&given_up), Some(CollectionFailure::Forbidden));
        assert!(given_up.to_string().starts_with("forbidden: "));
    }
    for refused in [public_refusal(403), owner_refusal(StatusCode::FORBIDDEN)] {
        let waiting = give_up_or_wait(&started(Duration::hours(1)), refused);
        assert_eq!(delay_of(&waiting), Some(1800));
    }
}

#[test]
fn a_refusal_on_a_later_page_waits_instead_of_dropping_the_pass() {
    for refused in [
        public_refusal(404),
        public_refusal(403),
        owner_refusal(StatusCode::NOT_FOUND),
        owner_refusal(StatusCode::FORBIDDEN),
    ] {
        let waiting = give_up_or_wait(&past_the_first_page(Duration::hours(7)), refused);
        assert_eq!(delay_of(&waiting), Some(1800));
    }
}

#[test]
fn a_rate_limited_collection_never_gives_up() {
    let old = started(Duration::hours(7));
    for limited in [
        public_refusal(429),
        owner_refusal(StatusCode::TOO_MANY_REQUESTS),
    ] {
        assert_eq!(delay_of(&give_up_or_wait(&old, limited)), Some(60));
    }
    let paced = owner_error(PlaylistReadError::Paced {
        retry_after_seconds: 7,
    });
    assert_eq!(delay_of(&give_up_or_wait(&old, paced)), Some(7));
    let next_page = postpone(1, anyhow::anyhow!("collection has another page"));
    assert_eq!(delay_of(&give_up_or_wait(&old, next_page)), Some(1));
    let unreachable = public_error(sc_transport::ScError::unreachable("connection reset"));
    assert!(give_up_or_wait(&old, unreachable).is_retryable());
}

#[sqlx::test(migrations = false)]
async fn an_owner_only_collection_of_a_signed_out_owner_fails_as_auth_required(
    pool: PgPool,
) -> anyhow::Result<()> {
    let (job, mut payload, writer) = setup(&pool).await?;
    let handler = CatalogCollectionHandler {
        public: Arc::new(PublicCatalogReader::offline(pool.clone())),
        remote: CatalogRemote::offline(pool.clone())?,
        writer,
        pool: pool.clone(),
    };
    state::begin(&pool, &job, &payload).await?;
    state::abandon_apiv2(&pool, &job, &payload).await?;
    handler.refresh(&job, payload.clone()).await?;
    assert!(synced_at(&pool, "liked-tracks").await?.is_some());

    payload.collection = CatalogCollection::LikedPlaylists;
    let error = handler
        .refresh(&job, payload)
        .await
        .err()
        .ok_or_else(|| anyhow::anyhow!("a signed-out owner cannot finish the pass"))?;

    assert_eq!(failure_of(&error), Some(CollectionFailure::AuthRequired));
    assert_eq!(synced_at(&pool, "liked-playlists").await?, None);
    Ok(())
}

async fn synced_at(
    pool: &PgPool,
    collection: &str,
) -> anyhow::Result<Option<chrono::DateTime<Utc>>> {
    Ok(sqlx::query_scalar(
        "SELECT synced_at FROM catalog_collection_sync
         WHERE subject_id = '42' AND collection = $1 AND scope = 'owner'",
    )
    .bind(collection)
    .fetch_one(pool)
    .await?)
}
