use catalog_normalize::normalize_title;
use chrono::{DateTime, NaiveDate, Utc};
use serde_json::Value;
use sqlx::PgConnection;

use uuid::Uuid;

use super::ActionError;

const PLAYLIST_ADD_WEIGHT: f64 = 0.9;

pub async fn finalize_create(
    connection: &mut PgConnection,
    mutation_id: Uuid,
    user_id: &str,
    remote_result: &Value,
) -> Result<(), ActionError> {
    let playlist = CreatedPlaylist::parse(remote_result)?;
    sqlx::query_file!(
        "queries/sync_queue/actions/upsert_created_playlist.sql",
        &playlist.urn,
        &playlist.sc_playlist_id,
        &playlist.title,
        &playlist.title_normalized,
        &playlist.description,
        &playlist.genre,
        &playlist.tags,
        &playlist.artwork_url,
        &playlist.permalink_url,
        &playlist.owner_sc_user_id,
        &playlist.owner_urn,
        &playlist.owner_username,
        playlist.track_count,
        playlist.duration_ms,
        &playlist.playlist_type,
        &playlist.kind,
        &playlist.sharing,
        playlist.release_year,
        &playlist.release_date,
        &playlist.label_name,
        playlist.likes_count,
        playlist.reposts_count,
        &playlist.created_at,
        &playlist.last_modified
    )
    .execute(&mut *connection)
    .await?;
    if let Some(track_ids) = &playlist.tracks {
        sqlx::query_file!(
            "queries/sync_queue/actions/delete_created_playlist_tracks.sql",
            &playlist.urn
        )
        .execute(&mut *connection)
        .await?;
        if !track_ids.is_empty() {
            sqlx::query_file!(
                "queries/sync_queue/actions/insert_created_playlist_tracks.sql",
                &playlist.urn,
                track_ids
            )
            .execute(&mut *connection)
            .await?;
        }
        record_playlist_adds(connection, mutation_id, user_id, track_ids).await?;
    }
    sqlx::query_file!(
        "queries/sync_queue/actions/ensure_created_playlist_state.sql",
        &playlist.urn
    )
    .execute(&mut *connection)
    .await?;
    sqlx::query_file!(
        "queries/sync_queue/actions/upsert_owned_playlist.sql",
        user_id,
        &playlist.urn
    )
    .execute(connection)
    .await?;
    Ok(())
}

async fn record_playlist_adds(
    connection: &mut PgConnection,
    mutation_id: Uuid,
    user_id: &str,
    track_ids: &[String],
) -> Result<(), ActionError> {
    let (event_ids, tracks) = playlist_add_events(mutation_id, track_ids);
    if tracks.is_empty() {
        return Ok(());
    }
    sqlx::query_file!(
        "queries/sync_queue/actions/record_created_playlist_adds.sql",
        mutation_id,
        user_id,
        &event_ids,
        &tracks,
        PLAYLIST_ADD_WEIGHT,
        &catalog_ingest::user_id_variants(user_id)
    )
    .execute(connection)
    .await?;
    Ok(())
}

fn playlist_add_events(mutation_id: Uuid, track_ids: &[String]) -> (Vec<Uuid>, Vec<String>) {
    let mut tracks: Vec<String> = Vec::with_capacity(track_ids.len());
    for track_id in track_ids {
        if !tracks.contains(track_id) {
            tracks.push(track_id.clone());
        }
    }
    let event_ids = tracks
        .iter()
        .map(|track_id| Uuid::new_v5(&mutation_id, track_id.as_bytes()))
        .collect();
    (event_ids, tracks)
}

struct CreatedPlaylist {
    urn: String,
    sc_playlist_id: String,
    title: String,
    title_normalized: String,
    description: String,
    genre: String,
    tags: Vec<String>,
    artwork_url: String,
    permalink_url: String,
    owner_sc_user_id: String,
    owner_urn: String,
    owner_username: String,
    track_count: i32,
    duration_ms: i64,
    playlist_type: String,
    kind: String,
    sharing: String,
    release_year: i16,
    release_date: String,
    label_name: String,
    likes_count: i64,
    reposts_count: i64,
    created_at: String,
    last_modified: String,
    tracks: Option<Vec<String>>,
}

impl CreatedPlaylist {
    fn parse(value: &Value) -> Result<Self, ActionError> {
        let urn = string(value, "urn")
            .ok_or_else(|| ActionError::InvalidRemoteResult("created playlist urn is missing"))?;
        let title = string(value, "title")
            .ok_or_else(|| ActionError::InvalidRemoteResult("created playlist title is missing"))?;
        let owner = value.get("user");
        let owner_urn = owner
            .and_then(|owner| string(owner, "urn"))
            .unwrap_or_default();
        let owner_sc_user_id = owner
            .and_then(|owner| string(owner, "id"))
            .unwrap_or_else(|| extract_sc_id(&owner_urn).to_owned());
        let tracks = value.get("tracks").and_then(Value::as_array).map(|tracks| {
            tracks
                .iter()
                .filter_map(|track| {
                    string(track, "urn")
                        .map(|urn| extract_sc_id(&urn).to_owned())
                        .or_else(|| string(track, "id"))
                })
                .collect::<Vec<_>>()
        });
        let release_date = extract_date(value);
        let release_year = value
            .get("release_year")
            .and_then(Value::as_i64)
            .and_then(|year| i16::try_from(year).ok())
            .or_else(|| release_date.and_then(|date| date.format("%Y").to_string().parse().ok()))
            .unwrap_or_default();
        Ok(Self {
            sc_playlist_id: extract_sc_id(&urn).to_owned(),
            title_normalized: normalize_title(&title),
            description: string(value, "description").unwrap_or_default(),
            genre: string(value, "genre").unwrap_or_default(),
            tags: string(value, "tag_list")
                .unwrap_or_default()
                .split_whitespace()
                .map(str::to_owned)
                .collect(),
            artwork_url: string(value, "artwork_url").unwrap_or_default(),
            permalink_url: string(value, "permalink_url").unwrap_or_default(),
            owner_username: owner
                .and_then(|owner| string(owner, "username"))
                .unwrap_or_default(),
            track_count: integer(value, "track_count")
                .and_then(|count| i32::try_from(count).ok())
                .unwrap_or_else(|| {
                    tracks
                        .as_ref()
                        .and_then(|tracks| i32::try_from(tracks.len()).ok())
                        .unwrap_or_default()
                }),
            duration_ms: integer(value, "duration").unwrap_or(-1),
            playlist_type: string(value, "playlist_type").unwrap_or_default(),
            kind: string(value, "kind").unwrap_or_default(),
            sharing: string(value, "sharing").unwrap_or_else(|| "public".to_owned()),
            release_year,
            release_date: release_date
                .map(|date| date.format("%Y-%m-%d").to_string())
                .unwrap_or_default(),
            label_name: string(value, "label_name").unwrap_or_default(),
            likes_count: integer(value, "likes_count").unwrap_or(-1),
            reposts_count: integer(value, "reposts_count")
                .or_else(|| integer(value, "repost_count"))
                .unwrap_or(-1),
            created_at: timestamp(value, "created_at"),
            last_modified: timestamp(value, "last_modified"),
            urn,
            title,
            owner_sc_user_id,
            owner_urn,
            tracks,
        })
    }
}

fn string(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(|value| match value {
            Value::String(value) => Some(value.trim().to_owned()),
            Value::Number(value) => Some(value.to_string()),
            _ => None,
        })
        .filter(|value| !value.is_empty())
}

fn integer(value: &Value, key: &str) -> Option<i64> {
    value.get(key).and_then(Value::as_i64)
}

fn timestamp(value: &Value, key: &str) -> String {
    string(value, key)
        .and_then(|value| parse_timestamp(&value))
        .map(|value| value.to_rfc3339())
        .unwrap_or_default()
}

fn parse_timestamp(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|value| value.with_timezone(&Utc))
        .or_else(|| {
            DateTime::parse_from_str(value, "%Y/%m/%d %H:%M:%S %z")
                .ok()
                .map(|value| value.with_timezone(&Utc))
        })
}

fn extract_date(value: &Value) -> Option<NaiveDate> {
    ["release_date", "display_date", "created_at"]
        .iter()
        .filter_map(|key| string(value, key))
        .find_map(|value| {
            value
                .get(..10)
                .and_then(|date| NaiveDate::parse_from_str(date, "%Y-%m-%d").ok())
        })
}

fn extract_sc_id(value: &str) -> &str {
    value.rsplit(':').next().unwrap_or(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn created_playlist_uses_the_shared_catalog_key() {
        assert_eq!(normalize_title("Thé ᴍᴏᴏɴ"), "moon");
    }

    #[test]
    fn a_repeated_track_is_one_addition_with_a_stable_identity() {
        let mutation = Uuid::now_v7();
        let tracks = ["7".to_owned(), "8".to_owned(), "7".to_owned()];
        let (first_ids, first_tracks) = playlist_add_events(mutation, &tracks);
        let (second_ids, _) = playlist_add_events(mutation, &tracks);
        assert_eq!(first_tracks, ["7", "8"]);
        assert_eq!(first_ids, second_ids);
        assert_ne!(first_ids[0], first_ids[1]);
    }

    async fn playlist_adds(pool: &sqlx::PgPool) -> anyhow::Result<Vec<(String, String, String)>> {
        Ok(sqlx::query_as(
            "SELECT sc_user_id, sc_track_id, to_char(created_at, 'YYYY-MM-DD\"T\"HH24:MI:SS')
             FROM user_events WHERE event_type = 'playlist_add' ORDER BY sc_track_id",
        )
        .fetch_all(pool)
        .await?)
    }

    #[sqlx::test(migrations = false)]
    async fn a_created_playlist_records_its_tracks_as_added_when_the_user_asked(
        pool: sqlx::PgPool,
    ) -> anyhow::Result<()> {
        crate::db::migrations::run_core(&pool, None).await?;
        let mutation: Uuid = sqlx::query_scalar(
            "INSERT INTO sync_queue (user_id, action_type, target_urn, created_at)
             VALUES ('17', 'playlist_create', 'new:x', '2026-03-04T05:06:07Z') RETURNING id",
        )
        .fetch_one(&pool)
        .await?;
        sqlx::query(
            "INSERT INTO disliked_tracks (sc_user_id, sc_track_id) VALUES ('soundcloud:users:17', '9')",
        )
        .execute(&pool)
        .await?;
        let created = serde_json::json!({
            "urn": "soundcloud:playlists:55",
            "title": "Mine",
            "user": {"id": 17, "urn": "soundcloud:users:17", "username": "me"},
            "created_at": "2026-09-24T10:00:00Z",
            "tracks": [{"urn": "soundcloud:tracks:7"}, {"id": 8}, {"id": 9}]
        });
        for _ in 0..2 {
            let mut tx = pool.begin().await?;
            finalize_create(&mut tx, mutation, "17", &created).await?;
            tx.commit().await?;
        }
        assert_eq!(
            playlist_adds(&pool).await?,
            [
                (
                    "17".to_owned(),
                    "7".to_owned(),
                    "2026-03-04T05:06:07".to_owned()
                ),
                (
                    "17".to_owned(),
                    "8".to_owned(),
                    "2026-03-04T05:06:07".to_owned()
                ),
            ]
        );
        Ok(())
    }

    #[sqlx::test(migrations = false)]
    async fn a_created_playlist_without_tracks_adds_nothing(
        pool: sqlx::PgPool,
    ) -> anyhow::Result<()> {
        crate::db::migrations::run_core(&pool, None).await?;
        let mutation: Uuid = sqlx::query_scalar(
            "INSERT INTO sync_queue (user_id, action_type, target_urn) VALUES ('17', 'playlist_create', 'new:y') RETURNING id",
        )
        .fetch_one(&pool)
        .await?;
        let created = serde_json::json!({
            "urn": "soundcloud:playlists:56",
            "title": "Empty",
            "user": {"id": 17, "urn": "soundcloud:users:17", "username": "me"}
        });
        let mut tx = pool.begin().await?;
        finalize_create(&mut tx, mutation, "17", &created).await?;
        tx.commit().await?;
        assert!(playlist_adds(&pool).await?.is_empty());
        Ok(())
    }
}
