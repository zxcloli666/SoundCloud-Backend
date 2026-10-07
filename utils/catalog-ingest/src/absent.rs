use std::collections::HashSet;

use serde_json::Value;
use sqlx::PgPool;

use crate::payload::ScTrackFields;
use crate::playlist::ScPlaylistFields;
use crate::user::ScUserFields;
use crate::{Observation, TrackPriority};

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AbsentInserted {
    pub tracks: Vec<String>,
    pub playlists: Vec<String>,
    pub users: Vec<String>,
}

pub async fn insert_absent_tracks(
    pool: &PgPool,
    payloads: &[Value],
    observation: Observation,
) -> Result<AbsentInserted, sqlx::Error> {
    let tracks = unique(
        payloads.iter().filter_map(ScTrackFields::from_sc),
        |track| track.sc_track_id.clone(),
    );
    let uploaders = payloads
        .iter()
        .filter_map(|payload| payload.get("user"))
        .cloned()
        .collect::<Vec<_>>();
    let users = insert_users(pool, &uploaders, observation).await?;
    let ids: Vec<String> = tracks
        .iter()
        .map(|track| track.sc_track_id.clone())
        .collect();
    let missing: HashSet<String> = sqlx::query_file_scalar!("queries/tracks/missing.sql", &ids)
        .fetch_all(pool)
        .await?
        .into_iter()
        .collect();
    let rows: Vec<Value> = tracks
        .iter()
        .filter(|track| missing.contains(&track.sc_track_id))
        .map(ScTrackFields::row)
        .collect();
    let inserted = if rows.is_empty() {
        Vec::new()
    } else {
        sqlx::query_file_scalar!(
            "queries/tracks/insert_absent.sql",
            Value::Array(rows),
            TrackPriority::Discovery.as_i16(),
            observation.sequence()
        )
        .fetch_all(pool)
        .await?
    };
    Ok(AbsentInserted {
        tracks: inserted,
        users,
        ..AbsentInserted::default()
    })
}

pub async fn insert_absent_playlists(
    pool: &PgPool,
    payloads: &[Value],
    observation: Observation,
) -> Result<AbsentInserted, sqlx::Error> {
    let playlists = unique(
        payloads
            .iter()
            .filter_map(|payload| Some((ScPlaylistFields::from_sc(payload)?, payload))),
        |(playlist, _)| playlist.urn().to_owned(),
    );
    let owners = payloads
        .iter()
        .filter_map(|payload| payload.get("user"))
        .cloned()
        .collect::<Vec<_>>();
    let users = insert_users(pool, &owners, observation).await?;
    let urns: Vec<String> = playlists
        .iter()
        .map(|(playlist, _)| playlist.urn().to_owned())
        .collect();
    let missing: HashSet<String> = sqlx::query_file_scalar!("queries/playlists/missing.sql", &urns)
        .fetch_all(pool)
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
        sqlx::query_file_scalar!(
            "queries/playlists/insert_absent.sql",
            Value::Array(rows),
            observation.sequence()
        )
        .fetch_all(pool)
        .await?
    };
    Ok(AbsentInserted {
        playlists: inserted,
        users,
        ..AbsentInserted::default()
    })
}

pub async fn insert_absent_users(
    pool: &PgPool,
    payloads: &[Value],
    observation: Observation,
) -> Result<AbsentInserted, sqlx::Error> {
    Ok(AbsentInserted {
        users: insert_users(pool, payloads, observation).await?,
        ..AbsentInserted::default()
    })
}

async fn insert_users(
    pool: &PgPool,
    payloads: &[Value],
    observation: Observation,
) -> Result<Vec<String>, sqlx::Error> {
    let users = unique(payloads.iter().filter_map(ScUserFields::from_sc), |user| {
        user.sc_user_id().to_owned()
    });
    if users.is_empty() {
        return Ok(Vec::new());
    }
    let ids: Vec<String> = users
        .iter()
        .map(|user| user.sc_user_id().to_owned())
        .collect();
    let missing: HashSet<String> = sqlx::query_file_scalar!("queries/users/missing.sql", &ids)
        .fetch_all(pool)
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
    sqlx::query_file_scalar!(
        "queries/users/insert_absent.sql",
        Value::Array(rows),
        observation.sequence()
    )
    .fetch_all(pool)
    .await
}

fn unique<T>(items: impl Iterator<Item = T>, key: impl Fn(&T) -> String) -> Vec<T> {
    let mut seen = HashSet::new();
    items.filter(|item| seen.insert(key(item))).collect()
}
