use backend_contracts::{CatalogCollection, CatalogCollectionPayload};
use chrono::{DateTime, Utc};
use sqlx::PgConnection;

use super::state::Snapshot;
use crate::queue::{JobError, JobResult};

enum Shape {
    User {
        table: &'static str,
        key: &'static str,
        wanted: bool,
    },
    Audience,
    Comments,
}

fn shape(collection: CatalogCollection) -> Shape {
    let user = |table, key, wanted| Shape::User { table, key, wanted };
    match collection {
        CatalogCollection::LikedTracks => user("user_likes_tracks", "sc_track_id", true),
        CatalogCollection::LikedPlaylists => user("user_likes_playlists", "playlist_urn", true),
        CatalogCollection::Followings => user("user_followings", "target_user_urn", true),
        CatalogCollection::Followers => user("user_followers", "target_user_urn", false),
        CatalogCollection::OwnedTracks => user("user_owned_tracks", "sc_track_id", false),
        CatalogCollection::OwnedPlaylists => user("user_owned_playlists", "playlist_urn", false),
        CatalogCollection::TrackFavoriters
        | CatalogCollection::TrackReposters
        | CatalogCollection::PlaylistReposters => Shape::Audience,
        CatalogCollection::TrackComments => Shape::Comments,
    }
}

pub(super) async fn persist_comments(
    connection: &mut PgConnection,
    payload: &CatalogCollectionPayload,
    snapshot: &Snapshot,
    items: &[serde_json::Value],
) -> JobResult {
    sqlx::query_file!(
        "queries/catalog_collection/comments_insert.sql",
        &payload.subject_id,
        snapshot.started_at,
        snapshot.item_count,
        serde_json::Value::Array(items.to_vec())
    )
    .execute(connection)
    .await
    .map_err(JobError::retryable)?;
    Ok(())
}

fn subject_urn(payload: &CatalogCollectionPayload) -> String {
    payload.collection.subject().urn(&payload.subject_id)
}

pub(super) async fn persist(
    connection: &mut PgConnection,
    payload: &CatalogCollectionPayload,
    snapshot: &Snapshot,
    keys: &[String],
) -> JobResult {
    let (table, key, wanted) = match shape(payload.collection) {
        Shape::User { table, key, wanted } => (table, key, wanted),
        Shape::Audience => {
            sqlx::query_file!(
                "queries/catalog_collection/audience_insert.sql",
                subject_urn(payload),
                payload.collection.as_str(),
                snapshot.started_at,
                snapshot.item_count,
                keys
            )
            .execute(connection)
            .await
            .map_err(JobError::retryable)?;
            return Ok(());
        }
        Shape::Comments => {
            return Err(JobError::permanent(anyhow::anyhow!(
                "comments use their own mirror"
            )));
        }
    };
    let (wanted_column, wanted_value) = if wanted {
        (", wanted_state", ", true")
    } else {
        ("", "")
    };
    let delete_guard = if payload.collection == CatalogCollection::OwnedPlaylists {
        "AND NOT EXISTS (SELECT 1 FROM sync_queue q WHERE q.user_id = ANY($5) AND q.action_type = 'playlist_delete' AND q.target_urn = item.key)
         AND NOT EXISTS (SELECT 1 FROM playlists p WHERE p.urn = item.key AND p.deleted_at IS NOT NULL)"
    } else if payload.collection == CatalogCollection::OwnedTracks {
        "AND NOT EXISTS (SELECT 1 FROM sync_queue q WHERE q.user_id = ANY($5) AND q.action_type = 'track_delete' AND q.target_urn = 'soundcloud:tracks:' || item.key)
         AND NOT EXISTS (SELECT 1 FROM tracks t WHERE t.sc_track_id = item.key AND t.deleted_at IS NOT NULL)"
    } else {
        ""
    };
    let sql = format!("INSERT INTO {table} (user_id, {key}, progress, synced_at, created_at{wanted_column})
        SELECT $1, item.key, false, $3, $3 - ($4 + item.ordinal)::double precision * interval '1 microsecond'{wanted_value}
        FROM unnest($2::text[]) WITH ORDINALITY AS item(key, ordinal)
        WHERE NOT EXISTS (SELECT 1 FROM {table} existing WHERE existing.user_id = ANY($5) AND existing.{key} = item.key)
        {delete_guard}
        ON CONFLICT (user_id, {key}) DO NOTHING");
    sqlx::query(&sql)
        .bind(&payload.subject_id)
        .bind(keys)
        .bind(snapshot.started_at)
        .bind(snapshot.item_count)
        .bind(catalog_ingest::user_id_variants(&payload.subject_id))
        .execute(connection)
        .await
        .map_err(JobError::retryable)?;
    Ok(())
}

pub(super) async fn record_like_times(
    connection: &mut PgConnection,
    payload: &CatalogCollectionPayload,
    liked_at: &[(String, DateTime<Utc>)],
) -> JobResult {
    if payload.collection != CatalogCollection::LikedTracks || liked_at.is_empty() {
        return Ok(());
    }
    let (tracks, times): (Vec<String>, Vec<DateTime<Utc>>) = liked_at.iter().cloned().unzip();
    sqlx::query_file!(
        "queries/catalog_collection/liked_at.sql",
        &catalog_ingest::user_id_variants(&payload.subject_id),
        &tracks,
        &times
    )
    .execute(connection)
    .await
    .map_err(JobError::retryable)?;
    Ok(())
}

pub(super) async fn reconcile(
    connection: &mut PgConnection,
    payload: &CatalogCollectionPayload,
    snapshot: &Snapshot,
) -> JobResult {
    if payload.collection == CatalogCollection::Followers {
        sqlx::query_file!(
            "queries/catalog_collection/reconcile_followers.sql",
            &payload.subject_id,
            snapshot.started_at,
            snapshot.snapshot_id
        )
        .execute(connection)
        .await
        .map_err(JobError::retryable)?;
        return Ok(());
    }
    if let Shape::Audience = shape(payload.collection) {
        sqlx::query_file!(
            "queries/catalog_collection/audience_reconcile.sql",
            subject_urn(payload),
            payload.collection.as_str(),
            snapshot.started_at,
            &payload.subject_id,
            snapshot.snapshot_id
        )
        .execute(connection)
        .await
        .map_err(JobError::retryable)?;
        return Ok(());
    }
    if let Shape::Comments = shape(payload.collection) {
        sqlx::query_file!(
            "queries/catalog_collection/comments_reconcile.sql",
            &payload.subject_id,
            snapshot.started_at,
            snapshot.snapshot_id
        )
        .execute(connection)
        .await
        .map_err(JobError::retryable)?;
        return Ok(());
    }
    if !payload.owner {
        return Ok(());
    }
    let Shape::User { table, key, wanted } = shape(payload.collection) else {
        return Ok(());
    };
    let wanted_filter = if wanted {
        "AND m.wanted_state = true"
    } else {
        ""
    };
    let sql = format!("DELETE FROM {table} m
        WHERE m.user_id = ANY($1) {wanted_filter} AND m.progress = false
        AND m.synced_at < $2 - interval '5 minutes' AND m.created_at < $2 - interval '5 minutes'
        AND NOT EXISTS (SELECT 1 FROM catalog_collection_seen s
            WHERE s.subject_id = $3 AND s.collection = $4 AND s.scope = 'owner'
              AND s.snapshot_id = $5 AND s.entity_key = m.{key})
        AND NOT EXISTS (SELECT 1 FROM sync_queue q WHERE q.user_id = ANY($1)
            AND (q.target_urn = m.{key} OR q.target_urn = 'soundcloud:tracks:' || m.{key}))
        AND (SELECT count(*) * 2 FROM catalog_collection_seen s
             WHERE s.subject_id = $3 AND s.collection = $4 AND s.scope = 'owner' AND s.snapshot_id = $5)
            >= (SELECT count(*) FROM {table} m WHERE m.user_id = ANY($1) {wanted_filter} AND m.progress = false)");
    sqlx::query(&sql)
        .bind(catalog_ingest::user_id_variants(&payload.subject_id))
        .bind(snapshot.started_at)
        .bind(&payload.subject_id)
        .bind(payload.collection.as_str())
        .bind(snapshot.snapshot_id)
        .execute(connection)
        .await
        .map_err(JobError::retryable)?;
    Ok(())
}
