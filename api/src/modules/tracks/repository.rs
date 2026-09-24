use chrono::{DateTime, NaiveDate, Utc};
use serde_json::{Map, Value, json};
use sqlx::FromRow;
use sqlx::PgPool;
use uuid::Uuid;

use crate::common::sc_payload::parse_id_or_string;
use crate::error::AppResult;

#[derive(Debug, Clone, FromRow)]
#[allow(dead_code)]
pub struct TrackRow {
    pub id: Uuid,
    pub sc_track_id: String,
    pub urn: String,

    pub title: String,
    pub title_normalized: String,
    pub description: Option<String>,
    pub genre: Option<String>,
    pub tags: Vec<String>,
    pub duration_ms: i32,
    pub artwork_url: Option<String>,
    pub permalink_url: Option<String>,
    pub waveform_url: Option<String>,
    pub language: Option<String>,
    pub language_confidence: Option<f32>,
    pub isrc: Option<String>,
    pub metadata_artist: Option<String>,
    pub sharing: String,
    pub sc_metadata: Value,
    pub deleted_at: Option<DateTime<Utc>>,
    pub sc_created_at: Option<DateTime<Utc>>,
    pub sc_last_modified: Option<DateTime<Utc>>,
    pub release_year: Option<i16>,
    pub release_date: Option<NaiveDate>,

    pub uploader_sc_user_id: Option<String>,
    pub uploader_urn: Option<String>,
    pub uploader_username: Option<String>,
    pub uploader_avatar_url: Option<String>,

    pub primary_artist_id: Option<Uuid>,
    pub album_id: Option<Uuid>,
    pub album_position: Option<i16>,
    pub canonical_track_id: Option<Uuid>,
    pub cover_of_artist_id: Option<Uuid>,
    pub upload_kind: String,

    pub audio_fingerprint: Option<String>,
    pub quality_score: Option<f32>,
    pub play_count_sc: Option<i64>,
    pub likes_count_sc: Option<i64>,
    pub reposts_count_sc: Option<i64>,
    pub comments_count_sc: Option<i64>,

    pub enrich_state: String,
    pub enrich_attempts: i16,
    pub enrich_source: Option<String>,
    pub enrich_confidence: Option<f32>,
    pub enriched_at: Option<DateTime<Utc>>,

    pub index_state: String,
    pub index_priority: i16,
    pub index_attempts: i16,
    pub indexed_at: Option<DateTime<Utc>>,

    pub storage_state: String,
    pub storage_priority: i16,
    pub storage_quality: Option<String>,
    pub storage_attempts: i16,
    pub s3_verified_at: Option<DateTime<Utc>>,
    pub s3_missing_at: Option<DateTime<Utc>>,
    pub hq_upgrade_pending: bool,
    pub hq_upgrade_attempts: i16,
    pub hq_upgrade_last_at: Option<DateTime<Utc>>,

    pub needs_duration_resolve: bool,

    pub sc_synced_at: DateTime<Utc>,
    pub last_read_at: Option<DateTime<Utc>>,

    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub struct TrackRepository {
    pg: PgPool,
}

impl TrackRepository {
    pub fn new(pg: PgPool) -> Self {
        Self { pg }
    }

    pub async fn upsert_from_sc(
        &self,
        fields: &catalog_ingest::ScTrackFields,
        index_priority: catalog_ingest::TrackPriority,
        storage_priority: catalog_ingest::TrackPriority,
        observation: catalog_ingest::Observation,
    ) -> AppResult<catalog_ingest::IngestResult> {
        let result = catalog_ingest::upsert_from_sc(
            &self.pg,
            fields,
            index_priority,
            storage_priority,
            observation,
        )
        .await?;
        Ok(result)
    }

    pub async fn mark_too_long(&self, sc_track_id: &str) -> AppResult<()> {
        sqlx::query_file!("queries/tracks/mark_too_long.sql", sc_track_id)
            .execute(&self.pg)
            .await?;
        Ok(())
    }
}

pub fn project_to_sc_shape(row: &TrackRow, uploader_user: Option<&Value>) -> Value {
    let mut obj = row.sc_metadata.as_object().cloned().unwrap_or_default();
    obj.insert("kind".into(), Value::String("track".into()));
    obj.insert("id".into(), parse_id_or_string(&row.sc_track_id));
    obj.insert("urn".into(), Value::String(row.urn.clone()));
    obj.insert("title".into(), Value::String(row.title.clone()));
    if let Some(d) = &row.description {
        obj.insert("description".into(), Value::String(d.clone()));
    }
    if let Some(g) = &row.genre {
        obj.insert("genre".into(), Value::String(g.clone()));
    }
    obj.insert("tag_list".into(), Value::String(row.tags.join(" ")));
    obj.insert("duration".into(), json!(row.duration_ms));
    obj.insert("full_duration".into(), json!(row.duration_ms));
    if let Some(a) = &row.artwork_url {
        obj.insert("artwork_url".into(), Value::String(a.clone()));
    }
    if let Some(p) = &row.permalink_url {
        obj.insert("permalink_url".into(), Value::String(p.clone()));
    }
    if let Some(w) = &row.waveform_url {
        obj.insert("waveform_url".into(), Value::String(w.clone()));
    }
    obj.insert("sharing".into(), Value::String(row.sharing.clone()));
    if let Some(t) = row.sc_created_at {
        obj.insert("created_at".into(), Value::String(t.to_rfc3339()));
    }
    if let Some(t) = row.sc_last_modified {
        obj.insert("last_modified".into(), Value::String(t.to_rfc3339()));
    }
    if let Some(y) = row.release_year {
        obj.insert("release_year".into(), json!(y));
    }
    if let Some(d) = row.release_date {
        obj.insert("release_date".into(), Value::String(d.to_string()));
    }
    if let Some(l) = &row.language {
        obj.insert("language".into(), Value::String(l.clone()));
    }
    let mut publisher = Map::new();
    if let Some(isrc) = &row.isrc {
        obj.insert("isrc".into(), Value::String(isrc.clone()));
        publisher.insert("isrc".into(), Value::String(isrc.clone()));
    }
    if let Some(artist) = &row.metadata_artist {
        obj.insert("metadata_artist".into(), Value::String(artist.clone()));
        publisher.insert("artist".into(), Value::String(artist.clone()));
    }
    if !publisher.is_empty() {
        obj.insert("publisher_metadata".into(), Value::Object(publisher));
    }
    obj.insert(
        "playback_count".into(),
        row.play_count_sc.map(|v| json!(v)).unwrap_or(Value::Null),
    );
    obj.insert(
        "likes_count".into(),
        row.likes_count_sc.map(|v| json!(v)).unwrap_or(Value::Null),
    );
    obj.insert(
        "reposts_count".into(),
        row.reposts_count_sc
            .map(|v| json!(v))
            .unwrap_or(Value::Null),
    );
    obj.insert(
        "comment_count".into(),
        row.comments_count_sc
            .map(|v| json!(v))
            .unwrap_or(Value::Null),
    );

    let user = uploader_user.cloned().unwrap_or_else(|| {
        let mut u = Map::new();
        if let Some(id) = &row.uploader_sc_user_id {
            u.insert("id".into(), parse_id_or_string(id));
        }
        if let Some(urn) = &row.uploader_urn {
            u.insert("urn".into(), Value::String(urn.clone()));
        }
        if let Some(n) = &row.uploader_username {
            u.insert("username".into(), Value::String(n.clone()));
        }
        if let Some(a) = &row.uploader_avatar_url {
            u.insert("avatar_url".into(), Value::String(a.clone()));
        }
        u.insert("kind".into(), Value::String("user".into()));
        Value::Object(u)
    });
    obj.insert("user".into(), user);

    let mut meta = Map::new();
    meta.insert(
        "storage_state".into(),
        Value::String(row.storage_state.clone()),
    );
    if let Some(q) = &row.storage_quality {
        meta.insert("storage_quality".into(), Value::String(q.clone()));
    }
    meta.insert("index_state".into(), Value::String(row.index_state.clone()));
    meta.insert(
        "enrich_state".into(),
        Value::String(row.enrich_state.clone()),
    );
    obj.insert("_scd_meta".into(), Value::Object(meta));

    Value::Object(obj)
}

pub async fn project_many(pg: &PgPool, sc_track_ids: &[String]) -> AppResult<Vec<Option<Value>>> {
    project_many_filtered(pg, sc_track_ids, false).await
}

pub async fn project_many_public(
    pg: &PgPool,
    sc_track_ids: &[String],
) -> AppResult<Vec<Option<Value>>> {
    project_many_filtered(pg, sc_track_ids, true).await
}

async fn project_many_filtered(
    pg: &PgPool,
    sc_track_ids: &[String],
    public_only: bool,
) -> AppResult<Vec<Option<Value>>> {
    if sc_track_ids.is_empty() {
        return Ok(Vec::new());
    }
    let rows: Vec<TrackRow> = if public_only {
        sqlx::query_file_as!(
            TrackRow,
            "queries/tracks/repository/project_many_public.sql",
            sc_track_ids
        )
        .fetch_all(pg)
        .await?
    } else {
        sqlx::query_file_as!(
            TrackRow,
            "queries/tracks/repository/project_many.sql",
            sc_track_ids
        )
        .fetch_all(pg)
        .await?
    };
    let by_id: std::collections::HashMap<String, TrackRow> = rows
        .into_iter()
        .map(|r| (r.sc_track_id.clone(), r))
        .collect();

    let uploader_ids: Vec<String> = by_id
        .values()
        .filter_map(|r| r.uploader_sc_user_id.clone())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();

    let users: std::collections::HashMap<String, Value> = if uploader_ids.is_empty() {
        Default::default()
    } else {
        sqlx::query_file!(
            "queries/tracks/repository/project_many_uploaders.sql",
            &uploader_ids
        )
        .fetch_all(pg)
        .await?
        .into_iter()
        .map(|r| (r.sc_user_id, r.u))
        .collect()
    };

    Ok(sc_track_ids
        .iter()
        .map(|id| {
            by_id.get(id).map(|row| {
                let uploader = row
                    .uploader_sc_user_id
                    .as_deref()
                    .and_then(|uid| users.get(uid));
                project_to_sc_shape(row, uploader)
            })
        })
        .collect())
}
