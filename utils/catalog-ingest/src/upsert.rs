use chrono::{DateTime, Utc};
use serde_json::json;
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;
use wt::{Mode, ObserveTracks, ObservedTrack};

use crate::Observation;
use crate::payload::ScTrackFields;

#[derive(Debug, Clone, Copy)]
pub enum TrackPriority {
    Like = 1,
    Playlist = 2,
    Played = 3,
    FreshDrop = 4,
    Discovery = 5,
}

impl TrackPriority {
    pub fn as_i16(self) -> i16 {
        self as i16
    }
}

pub struct IngestResult {
    pub id: Uuid,
    pub was_new: bool,
    pub metadata_applied: bool,
}

impl From<ObservedTrack> for IngestResult {
    fn from(track: ObservedTrack) -> Self {
        Self {
            id: track.id,
            was_new: track.was_new,
            metadata_applied: track.applied,
        }
    }
}

pub async fn bump_track_priority(
    pool: &PgPool,
    sc_track_id: &str,
    priority: TrackPriority,
) -> Result<(), sqlx::Error> {
    let observed_at = wt::now();
    wt::retry(|| async {
        let mut transaction = pool.begin().await?;
        bump_track_priority_at(&mut transaction, sc_track_id, priority, observed_at).await?;
        transaction.commit().await
    })
    .await
}

pub(crate) async fn bump_track_priority_at(
    connection: &mut PgConnection,
    sc_track_id: &str,
    priority: TrackPriority,
    observed_at: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    let bump = ObserveTracks {
        mode: Mode::Bump,
        observed_at,
        observation: 0,
        rows: vec![json!({
            "sc_track_id": sc_track_id,
            "index_priority": priority.as_i16(),
            "storage_priority": priority.as_i16(),
        })],
    };
    wt::call(connection, &bump).await?;
    Ok(())
}

pub async fn upsert_from_sc(
    pool: &PgPool,
    fields: &ScTrackFields,
    new_index_priority: TrackPriority,
    new_storage_priority: TrackPriority,
    observation: Observation,
) -> Result<IngestResult, sqlx::Error> {
    wt::retry(|| async {
        let mut transaction = pool.begin().await?;
        let result = upsert_track_in(
            &mut transaction,
            fields,
            new_index_priority,
            new_storage_priority,
            observation,
        )
        .await?;
        transaction.commit().await?;
        Ok(result)
    })
    .await
}

pub async fn upsert_track_in(
    connection: &mut PgConnection,
    fields: &ScTrackFields,
    new_index_priority: TrackPriority,
    new_storage_priority: TrackPriority,
    observation: Observation,
) -> Result<IngestResult, sqlx::Error> {
    upsert_track_at(
        connection,
        fields,
        new_index_priority,
        new_storage_priority,
        observation,
        wt::now(),
    )
    .await
}

pub(crate) async fn upsert_track_at(
    connection: &mut PgConnection,
    fields: &ScTrackFields,
    new_index_priority: TrackPriority,
    new_storage_priority: TrackPriority,
    observation: Observation,
    observed_at: DateTime<Utc>,
) -> Result<IngestResult, sqlx::Error> {
    let upsert = ObserveTracks {
        mode: Mode::Upsert,
        observed_at,
        observation: observation.sequence(),
        rows: vec![fields.row(new_index_priority, new_storage_priority)],
    };
    wt::call(connection, &upsert)
        .await?
        .into_iter()
        .next()
        .map(IngestResult::from)
        .ok_or(sqlx::Error::RowNotFound)
}
