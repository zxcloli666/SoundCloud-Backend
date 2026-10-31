use std::collections::HashSet;

use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::{PgConnection, PgPool};
use wt::{Mode, ObservePlaylists, ObserveTracks, ObserveUsers};

use crate::payload::ScTrackFields;
use crate::playlist::ScPlaylistFields;
use crate::sc_payload::without_nul;
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
    let observed_at = wt::now();
    wt::retry(|| async {
        let mut transaction = pool.begin().await?;
        let inserted =
            insert_absent_tracks_at(&mut transaction, payloads, observation, observed_at).await?;
        transaction.commit().await?;
        Ok(inserted)
    })
    .await
}

pub async fn insert_absent_playlists(
    pool: &PgPool,
    payloads: &[Value],
    observation: Observation,
) -> Result<AbsentInserted, sqlx::Error> {
    let observed_at = wt::now();
    wt::retry(|| async {
        let mut transaction = pool.begin().await?;
        let inserted =
            insert_absent_playlists_at(&mut transaction, payloads, observation, observed_at)
                .await?;
        transaction.commit().await?;
        Ok(inserted)
    })
    .await
}

pub async fn insert_absent_users(
    pool: &PgPool,
    payloads: &[Value],
    observation: Observation,
) -> Result<AbsentInserted, sqlx::Error> {
    let observed_at = wt::now();
    wt::retry(|| async {
        let mut transaction = pool.begin().await?;
        let inserted =
            insert_absent_users_at(&mut transaction, payloads, observation, observed_at).await?;
        transaction.commit().await?;
        Ok(inserted)
    })
    .await
}

pub(crate) async fn insert_absent_users_at(
    connection: &mut PgConnection,
    payloads: &[Value],
    observation: Observation,
    observed_at: DateTime<Utc>,
) -> Result<AbsentInserted, sqlx::Error> {
    Ok(AbsentInserted {
        users: insert_users(connection, payloads, observation, observed_at).await?,
        ..AbsentInserted::default()
    })
}

pub(crate) async fn insert_absent_tracks_at(
    connection: &mut PgConnection,
    payloads: &[Value],
    observation: Observation,
    observed_at: DateTime<Utc>,
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
    let users = insert_users(connection, &uploaders, observation, observed_at).await?;
    let ids: Vec<String> = tracks
        .iter()
        .map(|track| track.sc_track_id.clone())
        .collect();
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
        let absent = ObserveTracks {
            mode: Mode::Absent,
            observed_at,
            observation: observation.sequence(),
            rows,
        };
        wt::call(connection, &absent)
            .await?
            .into_iter()
            .map(|track| track.sc_track_id)
            .collect()
    };
    Ok(AbsentInserted {
        tracks: inserted,
        users,
        ..AbsentInserted::default()
    })
}

pub(crate) async fn insert_absent_playlists_at(
    connection: &mut PgConnection,
    payloads: &[Value],
    observation: Observation,
    observed_at: DateTime<Utc>,
) -> Result<AbsentInserted, sqlx::Error> {
    let payloads: Vec<Value> = payloads.iter().map(without_nul).collect();
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
    let users = insert_users(connection, &owners, observation, observed_at).await?;
    let urns: Vec<String> = playlists
        .iter()
        .map(|(playlist, _)| playlist.urn().to_owned())
        .collect();
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
        let absent = ObservePlaylists {
            mode: Mode::Absent,
            observed_at,
            observation: observation.sequence(),
            rows,
        };
        wt::call(connection, &absent)
            .await?
            .into_iter()
            .map(|playlist| playlist.urn)
            .collect()
    };
    Ok(AbsentInserted {
        playlists: inserted,
        users,
        ..AbsentInserted::default()
    })
}

async fn insert_users(
    connection: &mut PgConnection,
    payloads: &[Value],
    observation: Observation,
    observed_at: DateTime<Utc>,
) -> Result<Vec<String>, sqlx::Error> {
    let payloads: Vec<Value> = payloads.iter().map(without_nul).collect();
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
    let absent = ObserveUsers {
        mode: Mode::Absent,
        observed_at,
        observation: observation.sequence(),
        rows,
    };
    Ok(wt::call(connection, &absent)
        .await?
        .into_iter()
        .map(|user| user.sc_user_id)
        .collect())
}

fn unique<T>(items: impl Iterator<Item = T>, key: impl Fn(&T) -> String) -> Vec<T> {
    let mut seen = HashSet::new();
    items.filter(|item| seen.insert(key(item))).collect()
}
