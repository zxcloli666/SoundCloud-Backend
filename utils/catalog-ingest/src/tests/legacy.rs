use std::collections::HashSet;

use serde_json::Value;
use sqlx::PgConnection;
use uuid::Uuid;

use crate::payload::ScTrackFields;
use crate::playlist::ScPlaylistFields;
use crate::sc_payload::without_nul;
use crate::user::ScUserFields;
use crate::{AbsentInserted, IngestResult, Observation, TrackPriority};

pub(super) const OBSERVATION_GUARD: &str =
    include_str!("legacy/catalog_observation_is_current.sql");
const TRACK_UPSERT: &str = include_str!("legacy/tracks_upsert.sql");
const TRACK_BUMP: &str = include_str!("legacy/tracks_bump_priority.sql");
const TRACK_ABSENT: &str = include_str!("legacy/tracks_insert_absent.sql");
const PLAYLIST_LOCK: &str = include_str!("legacy/playlists_lock_membership.sql");
const PLAYLIST_UPSERT: &str = include_str!("legacy/playlists_upsert.sql");
const PLAYLIST_MEMBERSHIP: &str = include_str!("legacy/playlists_ensure_membership_state.sql");
const PLAYLIST_ABSENT: &str = include_str!("legacy/playlists_insert_absent.sql");
const USER_UPSERT: &str = include_str!("legacy/users_upsert.sql");
const USER_ABSENT: &str = include_str!("legacy/users_insert_absent.sql");
const USER_PROFILE: &str = include_str!("legacy/users_upsert_profile.sql");

pub(super) async fn bump_track_priority(
    connection: &mut PgConnection,
    sc_track_id: &str,
    priority: TrackPriority,
) -> Result<(), sqlx::Error> {
    sqlx::query(TRACK_BUMP)
        .bind(sc_track_id)
        .bind(priority.as_i16())
        .bind(priority.as_i16())
        .execute(connection)
        .await?;
    Ok(())
}

pub(super) async fn upsert_track(
    connection: &mut PgConnection,
    fields: &ScTrackFields,
    new_index_priority: TrackPriority,
    new_storage_priority: TrackPriority,
    observation: Observation,
) -> Result<IngestResult, sqlx::Error> {
    let row: Option<(Uuid, bool, bool)> = sqlx::query_as(TRACK_UPSERT)
        .bind(&fields.sc_track_id)
        .bind(&fields.urn)
        .bind(&fields.title)
        .bind(&fields.title_normalized)
        .bind(&fields.description)
        .bind(&fields.genre)
        .bind(&fields.tags)
        .bind(fields.duration_ms)
        .bind(&fields.artwork_url)
        .bind(&fields.permalink_url)
        .bind(&fields.waveform_url)
        .bind(&fields.language)
        .bind(&fields.isrc)
        .bind(&fields.metadata_artist)
        .bind(&fields.sharing)
        .bind(fields.sc_created_at)
        .bind(fields.sc_last_modified)
        .bind(fields.release_year)
        .bind(fields.release_date)
        .bind(&fields.uploader_sc_user_id)
        .bind(&fields.uploader_urn)
        .bind(&fields.uploader_username)
        .bind(&fields.uploader_avatar_url)
        .bind(fields.play_count_sc)
        .bind(fields.likes_count_sc)
        .bind(fields.reposts_count_sc)
        .bind(fields.comments_count_sc)
        .bind(fields.needs_duration_resolve)
        .bind(new_index_priority.as_i16())
        .bind(new_storage_priority.as_i16())
        .bind(fields.is_cover)
        .bind(observation.sequence())
        .bind(&fields.sc_metadata)
        .fetch_optional(&mut *connection)
        .await?;
    match row {
        Some((id, was_new, metadata_applied)) => Ok(IngestResult {
            id,
            was_new,
            metadata_applied,
        }),
        None => {
            sqlx::query(TRACK_BUMP)
                .bind(&fields.sc_track_id)
                .bind(new_index_priority.as_i16())
                .bind(new_storage_priority.as_i16())
                .execute(&mut *connection)
                .await?;
            let existing: (Uuid,) = sqlx::query_as("SELECT id FROM tracks WHERE sc_track_id = $1")
                .bind(&fields.sc_track_id)
                .fetch_one(&mut *connection)
                .await?;
            Ok(IngestResult {
                id: existing.0,
                was_new: false,
                metadata_applied: false,
            })
        }
    }
}

pub(super) async fn insert_absent_tracks(
    connection: &mut PgConnection,
    payloads: &[Value],
    observation: Observation,
) -> Result<AbsentInserted, sqlx::Error> {
    let tracks = unique(
        payloads.iter().filter_map(ScTrackFields::from_sc),
        |track| track.sc_track_id.clone(),
    );
    let uploaders: Vec<Value> = payloads
        .iter()
        .filter_map(|payload| payload.get("user"))
        .cloned()
        .collect();
    let users = insert_users(connection, &uploaders, observation).await?;
    let ids: Vec<String> = tracks.iter().map(|t| t.sc_track_id.clone()).collect();
    let missing: HashSet<String> = sqlx::query_file_scalar!("queries/tracks/missing.sql", &ids)
        .fetch_all(&mut *connection)
        .await?
        .into_iter()
        .collect();
    let rows: Vec<Value> = tracks
        .iter()
        .filter(|track| missing.contains(&track.sc_track_id))
        .map(|track| track.row(TrackPriority::Discovery, TrackPriority::Discovery))
        .collect();
    let inserted = if rows.is_empty() {
        Vec::new()
    } else {
        sqlx::query_scalar(TRACK_ABSENT)
            .bind(Value::Array(rows))
            .bind(TrackPriority::Discovery.as_i16())
            .bind(observation.sequence())
            .fetch_all(&mut *connection)
            .await?
    };
    Ok(AbsentInserted {
        tracks: inserted,
        users,
        ..AbsentInserted::default()
    })
}

pub(super) async fn insert_absent_playlists(
    connection: &mut PgConnection,
    payloads: &[Value],
    observation: Observation,
) -> Result<AbsentInserted, sqlx::Error> {
    let payloads: Vec<Value> = payloads.iter().map(without_nul).collect();
    let playlists = unique(
        payloads
            .iter()
            .filter_map(|payload| Some((ScPlaylistFields::from_sc(payload)?, payload))),
        |(playlist, _)| playlist.urn().to_owned(),
    );
    let owners: Vec<Value> = payloads
        .iter()
        .filter_map(|payload| payload.get("user"))
        .cloned()
        .collect();
    let users = insert_users(connection, &owners, observation).await?;
    let urns: Vec<String> = playlists.iter().map(|(p, _)| p.urn().to_owned()).collect();
    let missing: HashSet<String> = sqlx::query_file_scalar!("queries/playlists/missing.sql", &urns)
        .fetch_all(&mut *connection)
        .await?
        .into_iter()
        .collect();
    let rows: Vec<Value> = playlists
        .iter()
        .filter(|(playlist, _)| missing.contains(playlist.urn()))
        .map(|(playlist, payload)| playlist.row(payload))
        .collect();
    let inserted = if rows.is_empty() {
        Vec::new()
    } else {
        sqlx::query_scalar(PLAYLIST_ABSENT)
            .bind(Value::Array(rows))
            .bind(observation.sequence())
            .fetch_all(&mut *connection)
            .await?
    };
    Ok(AbsentInserted {
        playlists: inserted,
        users,
        ..AbsentInserted::default()
    })
}

pub(super) async fn insert_absent_users(
    connection: &mut PgConnection,
    payloads: &[Value],
    observation: Observation,
) -> Result<AbsentInserted, sqlx::Error> {
    Ok(AbsentInserted {
        users: insert_users(connection, payloads, observation).await?,
        ..AbsentInserted::default()
    })
}

async fn insert_users(
    connection: &mut PgConnection,
    payloads: &[Value],
    observation: Observation,
) -> Result<Vec<String>, sqlx::Error> {
    let payloads: Vec<Value> = payloads.iter().map(without_nul).collect();
    let users = unique(payloads.iter().filter_map(ScUserFields::from_sc), |user| {
        user.sc_user_id().to_owned()
    });
    if users.is_empty() {
        return Ok(Vec::new());
    }
    let ids: Vec<String> = users.iter().map(|u| u.sc_user_id().to_owned()).collect();
    let missing: HashSet<String> = sqlx::query_file_scalar!("queries/users/missing.sql", &ids)
        .fetch_all(&mut *connection)
        .await?
        .into_iter()
        .collect();
    let rows: Vec<Value> = users
        .iter()
        .filter(|user| missing.contains(user.sc_user_id()))
        .map(ScUserFields::row)
        .collect();
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    sqlx::query_scalar(USER_ABSENT)
        .bind(Value::Array(rows))
        .bind(observation.sequence())
        .fetch_all(connection)
        .await
}

pub(super) async fn upsert_playlist(
    connection: &mut PgConnection,
    payload: &Value,
    observation: Observation,
) -> Result<bool, sqlx::Error> {
    let payload = &without_nul(payload);
    let Some(fields) = ScPlaylistFields::from_sc(payload) else {
        return Ok(false);
    };
    sqlx::query(PLAYLIST_LOCK)
        .bind(&fields.urn)
        .fetch_optional(&mut *connection)
        .await?;
    let was_new: Option<bool> = sqlx::query_scalar(PLAYLIST_UPSERT)
        .bind(&fields.urn)
        .bind(&fields.sc_playlist_id)
        .bind(&fields.title)
        .bind(&fields.title_normalized)
        .bind(fields.description.as_deref())
        .bind(fields.genre.as_deref())
        .bind(&fields.tags)
        .bind(fields.artwork_url.as_deref())
        .bind(fields.permalink_url.as_deref())
        .bind(fields.owner_sc_user_id.as_deref())
        .bind(fields.owner_urn.as_deref())
        .bind(fields.owner_username.as_deref())
        .bind(fields.track_count)
        .bind(fields.duration_ms)
        .bind(fields.playlist_type.as_deref())
        .bind(fields.kind.as_deref())
        .bind(&fields.sharing)
        .bind(fields.release_year)
        .bind(fields.release_date)
        .bind(fields.label_name.as_deref())
        .bind(fields.likes_count_sc)
        .bind(fields.reposts_count_sc)
        .bind(fields.sc_created_at)
        .bind(fields.sc_last_modified)
        .bind(observation.sequence())
        .bind(crate::playlist_metadata::metadata_from_sc(payload))
        .fetch_optional(&mut *connection)
        .await?;
    sqlx::query(PLAYLIST_MEMBERSHIP)
        .bind(&fields.urn)
        .execute(&mut *connection)
        .await?;
    Ok(was_new.unwrap_or(false))
}

pub(super) async fn upsert_user(
    connection: &mut PgConnection,
    payload: &Value,
    observation: Observation,
) -> Result<bool, sqlx::Error> {
    let payload = &without_nul(payload);
    let Some(fields) = ScUserFields::from_sc(payload) else {
        return Err(sqlx::Error::Protocol(
            "invalid SoundCloud user payload".into(),
        ));
    };
    let was_new: Option<bool> = sqlx::query_scalar(USER_UPSERT)
        .bind(&fields.sc_user_id)
        .bind(&fields.urn)
        .bind(&fields.username)
        .bind(&fields.username_normalized)
        .bind(fields.full_name.as_deref())
        .bind(fields.first_name.as_deref())
        .bind(fields.last_name.as_deref())
        .bind(fields.permalink.as_deref())
        .bind(fields.permalink_url.as_deref())
        .bind(fields.avatar_url.as_deref())
        .bind(fields.country.as_deref())
        .bind(fields.city.as_deref())
        .bind(fields.description.as_deref())
        .bind(fields.verified)
        .bind(fields.followers_count)
        .bind(fields.followings_count)
        .bind(fields.tracks_count)
        .bind(fields.playlists_count)
        .bind(fields.reposts_count)
        .bind(fields.comments_count)
        .bind(fields.kind.as_deref())
        .bind(fields.sc_created_at)
        .bind(fields.sc_last_modified)
        .bind(observation.sequence())
        .bind(payload)
        .fetch_optional(&mut *connection)
        .await?;
    Ok(was_new.unwrap_or(false))
}

pub(super) async fn upsert_profile(
    connection: &mut PgConnection,
    user_id: &str,
    profile: &Value,
    observation: Observation,
) -> Result<(), sqlx::Error> {
    let profile = &without_nul(profile);
    if profile.get("urn").and_then(Value::as_str) != Some(crate::sc_ids::user_urn(user_id).as_str())
    {
        return Err(sqlx::Error::Protocol(
            "SoundCloud profile identity mismatch".into(),
        ));
    }
    upsert_user(connection, profile, observation).await?;
    sqlx::query(USER_PROFILE)
        .bind(user_id)
        .bind(profile)
        .bind(observation.sequence())
        .execute(connection)
        .await?;
    Ok(())
}

fn unique<T>(items: impl Iterator<Item = T>, key: impl Fn(&T) -> String) -> Vec<T> {
    let mut seen = HashSet::new();
    items.filter(|item| seen.insert(key(item))).collect()
}
