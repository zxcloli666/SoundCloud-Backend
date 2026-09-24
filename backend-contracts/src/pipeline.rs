use std::collections::BTreeMap;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};

use crate::reasons::{WorkerReason, WorkerStatus};
use crate::vector_store::{
    QUERY_VEC_LYRICS_DIMENSIONS, QUERY_VEC_MULAN_DIMENSIONS, TRACKS_CLAP_DIMENSIONS,
    TRACKS_COLLAB_DIMENSIONS, TRACKS_LYRICS_DIMENSIONS, TRACKS_MERT_DIMENSIONS,
    TRACKS_TASTE_DIMENSIONS,
};
use crate::worker_contract::done_duplicate_window_seconds;

pub const SC_TRACK_ID_PATTERN: &str = "^[1-9][0-9]{0,18}$";
pub const HTTP_URL_PATTERN: &str = "^https?://";
pub const LANGUAGE_PATTERN: &str = "^[a-z]{2}$";
pub const SHA256_HEX_PATTERN: &str = "^[0-9a-f]{64}$";
pub const MAX_URL_CHARS: u32 = 4096;
pub const MAX_TEXT_BYTES: u32 = 16_000;
pub const MAX_ENCODE_TEXT_BYTES: u32 = 512;
pub const MAX_REQUEST_ID_CHARS: u32 = 128;
pub const MAX_DETAIL_CHARS: u32 = 256;
pub const MAX_FINGERPRINT_CHARS: u32 = 64;
pub const MAX_RESOLVE_DESCRIPTION_CHARS: u32 = 4000;
pub const MAX_MATCH_CANDIDATES: u32 = 50;
pub const MAX_MESSAGE_BYTES: u32 = 900_000;
pub const MAX_UTF8_BYTES_KEYWORD: &str = "x-max-utf8-bytes";
pub const COLLAB_DATASET_VERSION: u32 = 2;
pub const TASTE_DATASET_VERSION: u32 = 1;

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
pub struct Producer {
    #[schemars(length(min = 1))]
    pub worker_id: String,
    pub build: String,
    pub models: BTreeMap<String, String>,
    #[schemars(schema_with = "nullable::<String>")]
    pub sync_version: Option<String>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
pub struct AudioIndexRequest {
    #[schemars(pattern(SC_TRACK_ID_PATTERN))]
    pub sc_track_id: String,
    #[schemars(pattern(HTTP_URL_PATTERN), length(max = MAX_URL_CHARS))]
    pub s3_url: String,
    #[schemars(range(min = 1))]
    pub upload_generation: i64,
    #[schemars(range(min = 1))]
    pub attempt: i64,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
pub struct AudioIndexResult {
    #[schemars(pattern(SC_TRACK_ID_PATTERN))]
    pub sc_track_id: String,
    #[schemars(range(min = 1))]
    pub upload_generation: i64,
    #[schemars(range(min = 1))]
    pub attempt: i64,
    pub status: WorkerStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "WorkerReason")]
    pub reason: Option<WorkerReason>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "String", length(max = MAX_DETAIL_CHARS))]
    pub detail: Option<String>,
    pub producer: Producer,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(
        with = "Vec<f32>",
        length(min = TRACKS_MERT_DIMENSIONS, max = TRACKS_MERT_DIMENSIONS)
    )]
    pub mert: Option<Vec<f32>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(
        with = "Vec<f32>",
        length(min = TRACKS_CLAP_DIMENSIONS, max = TRACKS_CLAP_DIMENSIONS)
    )]
    pub clap: Option<Vec<f32>>,
    #[schemars(length(max = MAX_FINGERPRINT_CHARS))]
    pub fingerprint: Option<String>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
pub struct LyricsEmbeddingRequest {
    #[schemars(pattern(SC_TRACK_ID_PATTERN))]
    pub sc_track_id: String,
    #[schemars(length(min = 1, max = MAX_REQUEST_ID_CHARS))]
    pub request_id: String,
    #[schemars(length(min = 1), extend("x-max-utf8-bytes" = MAX_TEXT_BYTES))]
    pub text: String,
    #[schemars(pattern(LANGUAGE_PATTERN))]
    pub language: Option<String>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
pub struct LyricsEmbeddingResult {
    #[schemars(pattern(SC_TRACK_ID_PATTERN))]
    pub sc_track_id: String,
    #[schemars(length(min = 1, max = MAX_REQUEST_ID_CHARS))]
    pub request_id: String,
    pub status: WorkerStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "WorkerReason")]
    pub reason: Option<WorkerReason>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "String", length(max = MAX_DETAIL_CHARS))]
    pub detail: Option<String>,
    pub producer: Producer,
    #[serde(default, rename = "vec", skip_serializing_if = "Option::is_none")]
    #[schemars(
        with = "Vec<f32>",
        length(min = TRACKS_LYRICS_DIMENSIONS, max = TRACKS_LYRICS_DIMENSIONS)
    )]
    pub vector: Option<Vec<f32>>,
    #[schemars(pattern(LANGUAGE_PATTERN))]
    pub language: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TranscriptionMode {
    Align,
}

impl TranscriptionMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Align => "align",
        }
    }
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
pub struct TranscriptionRequest {
    #[schemars(pattern(SC_TRACK_ID_PATTERN))]
    pub sc_track_id: String,
    #[schemars(range(min = 1))]
    pub upload_generation: i64,
    #[schemars(range(min = 1))]
    pub attempt: i64,
    #[schemars(pattern(HTTP_URL_PATTERN), length(max = MAX_URL_CHARS))]
    pub audio_url: String,
    #[schemars(length(min = 1), extend("x-max-utf8-bytes" = MAX_TEXT_BYTES))]
    pub reference_text: String,
    #[schemars(range(min = 1))]
    pub reference_lines_total: i64,
    #[schemars(schema_with = "nullable::<String>", pattern(LANGUAGE_PATTERN))]
    pub language: Option<String>,
    pub mode: TranscriptionMode,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
pub struct TranscriptionWord {
    #[schemars(range(min = 0))]
    pub line: u32,
    pub text: String,
    #[schemars(range(min = 0))]
    pub start_ms: u64,
    #[schemars(range(min = 0))]
    pub end_ms: u64,
    pub interpolated: bool,
    pub unplaced: bool,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
pub struct TranscriptionResult {
    #[schemars(pattern(SC_TRACK_ID_PATTERN))]
    pub sc_track_id: String,
    #[schemars(range(min = 1))]
    pub upload_generation: i64,
    #[schemars(range(min = 1))]
    pub attempt: i64,
    pub mode: TranscriptionMode,
    pub status: WorkerStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "WorkerReason")]
    pub reason: Option<WorkerReason>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "String", length(max = MAX_DETAIL_CHARS))]
    pub detail: Option<String>,
    #[schemars(schema_with = "synced_producer_schema")]
    pub producer: Producer,
    #[schemars(length(min = 1))]
    pub sync_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "f64", range(min = 0.0, max = 1.0))]
    pub confidence: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "f64", range(min = 0.0, max = 1.0))]
    pub placed_share: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "f64", range(min = 0.0, max = 1.0))]
    pub aligned_share: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "u32")]
    pub lines_total: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "u32")]
    pub lines_unplaced: Option<u32>,
    #[schemars(pattern(LANGUAGE_PATTERN))]
    pub language: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "String")]
    pub synced_lrc: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Vec<TranscriptionWord>")]
    pub words: Option<Vec<TranscriptionWord>>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EncodeModel {
    Mulan,
    Lyrics,
}

impl EncodeModel {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Mulan => "mulan",
            Self::Lyrics => "lyrics",
        }
    }

    pub const fn dimensions(self) -> u64 {
        match self {
            Self::Mulan => QUERY_VEC_MULAN_DIMENSIONS,
            Self::Lyrics => QUERY_VEC_LYRICS_DIMENSIONS,
        }
    }
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
pub struct EncodeRequest {
    pub model: EncodeModel,
    #[schemars(extend("x-max-utf8-bytes" = MAX_ENCODE_TEXT_BYTES))]
    pub text: String,
    #[schemars(pattern(SHA256_HEX_PATTERN))]
    pub hash: String,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
pub struct EncodeResult {
    pub model: EncodeModel,
    #[schemars(pattern(SHA256_HEX_PATTERN))]
    pub hash: String,
    pub status: WorkerStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "WorkerReason")]
    pub reason: Option<WorkerReason>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "String", length(max = MAX_DETAIL_CHARS))]
    pub detail: Option<String>,
    pub producer: Producer,
    #[schemars(schema_with = "encode_vector_schema")]
    pub vector: Option<Vec<f32>>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
pub struct CollabTrainRequest {
    #[schemars(length(min = 1))]
    pub object: String,
    #[schemars(extend("const" = COLLAB_DATASET_VERSION))]
    pub dataset_version: u32,
    #[schemars(extend("const" = TRACKS_COLLAB_DIMENSIONS))]
    pub dim: u64,
    #[schemars(range(min = 1))]
    pub min_count: u32,
    #[schemars(range(min = 1))]
    pub window: u32,
    #[schemars(range(min = 1))]
    pub epochs: u32,
    #[schemars(range(min = 1))]
    pub negative: u32,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
pub struct CollabTrainResult {
    #[schemars(length(min = 1))]
    pub input_object: String,
    pub status: WorkerStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "WorkerReason")]
    pub reason: Option<WorkerReason>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "String", length(max = MAX_DETAIL_CHARS))]
    pub detail: Option<String>,
    pub producer: Producer,
    pub trained: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "String", length(min = 1))]
    pub object: Option<String>,
    #[schemars(extend("const" = TRACKS_COLLAB_DIMENSIONS))]
    pub dim: u64,
    pub points_count: u64,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
pub struct TasteTrainRequest {
    #[schemars(length(min = 1))]
    pub object: String,
    #[schemars(extend("const" = TASTE_DATASET_VERSION))]
    pub dataset_version: u32,
    #[schemars(extend("const" = TRACKS_TASTE_DIMENSIONS))]
    pub dim: u64,
    #[schemars(range(min = 1))]
    pub epochs: u32,
    #[schemars(range(min = 1))]
    pub batch_size: u32,
    #[schemars(range(min = 1))]
    pub negatives: u32,
    #[schemars(range(min = 1))]
    pub seed: u32,
    #[schemars(schema_with = "nullable::<String>")]
    pub previous_version: Option<String>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
pub struct TasteBaselines {
    #[schemars(range(min = 0.0, max = 1.0))]
    pub popularity: f64,
    #[schemars(range(min = 0.0, max = 1.0))]
    pub item2vec: f64,
    #[schemars(range(min = 0.0, max = 1.0))]
    pub content: f64,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
pub struct TasteMetrics {
    #[schemars(range(min = 0.0, max = 1.0))]
    pub recall_at_50: f64,
    #[schemars(range(min = 0.0, max = 1.0))]
    pub ndcg_at_20: f64,
    #[schemars(range(min = 0.0, max = 1.0))]
    pub cold_recall_at_50: f64,
    #[schemars(range(min = 0.0, max = 1.0))]
    pub coverage_at_50: f64,
    pub baselines: TasteBaselines,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
pub struct TasteTrainResult {
    #[schemars(length(min = 1))]
    pub input_object: String,
    pub status: WorkerStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "WorkerReason")]
    pub reason: Option<WorkerReason>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "String", length(max = MAX_DETAIL_CHARS))]
    pub detail: Option<String>,
    pub producer: Producer,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "String", length(min = 1))]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "String", length(min = 1))]
    pub object: Option<String>,
    #[schemars(extend("const" = TRACKS_TASTE_DIMENSIONS))]
    pub dim: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "u64")]
    pub items_count: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "u64")]
    pub users_count: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "TasteMetrics")]
    pub metrics: Option<TasteMetrics>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
pub struct ResolveArtistRequest {
    #[schemars(length(min = 1))]
    pub title: String,
    pub uploader: Option<String>,
    pub metadata_artist: Option<String>,
    pub isrc: Option<String>,
    #[schemars(length(max = MAX_RESOLVE_DESCRIPTION_CHARS))]
    pub description: Option<String>,
    #[schemars(range(min = 0))]
    pub duration_ms: Option<i64>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RpcSource {
    Deterministic,
    Llm,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RpcError {
    Expired,
    InvalidRequest,
    Internal,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
pub struct ResolvedAlbum {
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "i32")]
    pub year: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "String")]
    pub primary_artist: Option<String>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
pub struct ResolveArtistData {
    #[schemars(schema_with = "nullable::<String>")]
    pub primary_artist: Option<String>,
    pub featured: Vec<String>,
    pub producers: Vec<String>,
    pub remixers: Vec<String>,
    #[schemars(schema_with = "nullable::<ResolvedAlbum>")]
    pub album: Option<ResolvedAlbum>,
    #[schemars(range(min = 0.0, max = 1.0))]
    pub confidence: f64,
    pub source: RpcSource,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
pub struct MatchTarget {
    pub artist: String,
    pub title: String,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
pub struct MatchCandidate {
    #[schemars(range(max = u32::MAX))]
    pub id: u32,
    pub artist: String,
    pub title: String,
    #[schemars(range(min = 0.0))]
    pub duration_sec: Option<f64>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
pub struct MatchTrackRequest {
    pub target: MatchTarget,
    #[schemars(length(min = 1, max = MAX_MATCH_CANDIDATES))]
    pub candidates: Vec<MatchCandidate>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
pub struct MatchTrackData {
    #[schemars(schema_with = "nullable::<u32>", range(max = u32::MAX))]
    pub match_id: Option<u32>,
    #[schemars(range(min = 0.0, max = 1.0))]
    pub confidence: f64,
    pub source: RpcSource,
}

fn nullable<T: JsonSchema>(generator: &mut SchemaGenerator) -> Schema {
    generator.subschema_for::<Option<T>>()
}

fn synced_producer_schema(generator: &mut SchemaGenerator) -> Schema {
    let producer = generator.subschema_for::<Producer>();
    json_schema!({
        "allOf": [
            producer,
            { "properties": { "sync_version": { "type": "string", "minLength": 1 } } }
        ]
    })
}

fn encode_vector_schema(_: &mut SchemaGenerator) -> Schema {
    let vector = |dimensions: u64| {
        json_schema!({
            "type": "array",
            "items": { "type": "number" },
            "minItems": dimensions,
            "maxItems": dimensions
        })
    };
    json_schema!({
        "oneOf": [
            vector(EncodeModel::Mulan.dimensions()),
            vector(EncodeModel::Lyrics.dimensions()),
            { "type": "null" }
        ]
    })
}

pub const COLLAB_OBJECT_PLACEHOLDER: &str = "{object}";
pub const COLLAB_VECTORS_OBJECT_TEMPLATE: &str = "{object}-vectors";

pub fn collab_vectors_object(input_object: &str) -> String {
    COLLAB_VECTORS_OBJECT_TEMPLATE.replace(COLLAB_OBJECT_PLACEHOLDER, input_object)
}

pub fn collab_vectors_suffix() -> &'static str {
    COLLAB_VECTORS_OBJECT_TEMPLATE
        .strip_prefix(COLLAB_OBJECT_PLACEHOLDER)
        .expect("the collab vectors template starts with the input object")
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StorageRejectionReason {
    DurationMismatch,
    TooShort,
    TooLong,
    #[default]
    #[serde(other)]
    Unknown,
}

impl StorageRejectionReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DurationMismatch => "duration_mismatch",
            Self::TooShort => "too_short",
            Self::TooLong => "too_long",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct StorageTrackRejected {
    pub sc_track_id: String,
    #[serde(default)]
    pub reason: StorageRejectionReason,
    #[serde(default)]
    pub actual_secs: Option<f64>,
    #[serde(default)]
    pub expected_duration_ms: Option<i64>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct StorageTrackUploaded {
    pub sc_track_id: String,
    pub storage_url: String,
    #[serde(default)]
    pub quality: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StreamDiscard {
    Old,
    New,
}

impl StreamDiscard {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Old => "old",
            Self::New => "new",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PipelineStreamSpec {
    pub name: &'static str,
    pub subjects: &'static [&'static str],
    pub work_queue: bool,
    pub max_age_seconds: u64,
    pub duplicate_window_seconds: u64,
    pub max_bytes: i64,
    pub discard: StreamDiscard,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ObjectStoreSpec {
    pub bucket: &'static str,
    pub max_age_seconds: Option<u64>,
}

pub const MSG_ID_HEADER: &str = "Nats-Msg-Id";
pub const REPLY_TO_HEADER: &str = "X-Reply-To";
pub const DEADLINE_HEADER: &str = "X-Deadline";
pub const WORKER_ID_HEADER: &str = "X-Worker-Id";
pub const WORKER_BUILD_HEADER: &str = "X-Worker-Build";
pub const DELIVERIES_HEADER: &str = "X-Deliveries";
pub const PUBLIC_NODE_HEADER: &str = "X-Public-Node";
pub const HEADERS: [(&str, &str); 7] = [
    ("msg_id", MSG_ID_HEADER),
    ("reply_to", REPLY_TO_HEADER),
    ("deadline", DEADLINE_HEADER),
    ("worker_id", WORKER_ID_HEADER),
    ("worker_build", WORKER_BUILD_HEADER),
    ("deliveries", DELIVERIES_HEADER),
    ("public_node", PUBLIC_NODE_HEADER),
];

pub const AI_RESOLVE_ARTIST: &str = "ai.rpc.resolve_artist";
pub const AI_MATCH_TRACK: &str = "ai.rpc.match_track";
pub const RESOLVE_ARTIST_WINDOW_SECONDS: u64 = 20;
pub const MATCH_TRACK_WINDOW_SECONDS: u64 = 10;

pub const INDEX_AUDIO: &str = "index.audio.new";
pub const EMBED_LYRICS: &str = "embed.lyrics.new";
pub const TRANSCRIBE_AUDIO: &str = "transcribe.audio.new";
pub const ENCODE_TEXT_NEW: &str = "encode.text.new";
pub const TRAIN_COLLAB: &str = "train.collab.new";
pub const TRAIN_TASTE: &str = "train.taste.new";

pub const DONE_INDEX_AUDIO: &str = "done.index_audio";
pub const DONE_EMBED_LYRICS: &str = "done.embed_lyrics";
pub const DONE_TRANSCRIBE: &str = "done.transcribe";
pub const DONE_TRAIN_COLLAB: &str = "done.train_collab";
pub const DONE_TRAIN_TASTE: &str = "done.train_taste";
pub const DONE_ENCODE: &str = "done.encode";

pub const WORKER_INVALID_SUBJECT: &str = "worker.invalid.<lane>";
pub const WORKER_STATUS_SUBJECT: &str = "worker.status.<worker_id>";
pub const WORKER_HEALTH_SUBJECT: &str = "worker.health.<worker_id>";
pub const WORKER_STATUS_WILDCARD: &str = "worker.status.>";

pub const COLLAB_DATA_BUCKET: &str = "COLLAB_DATA";
pub const TASTE_DATA_BUCKET: &str = "TASTE_DATA";
pub const TASTE_MODELS_BUCKET: &str = "TASTE_MODELS";
pub const STORAGE_TRACK_UPLOADED: &str = "storage.track_uploaded";
pub const STORAGE_TRACK_REJECTED: &str = "storage.track_rejected";

const HOUR: u64 = 60 * 60;
const DAY: u64 = 24 * HOUR;
const GIB: i64 = 1024 * 1024 * 1024;
pub const JOB_STREAM_MAX_BYTES: i64 = GIB;
pub const DONE_STREAM_MAX_BYTES: i64 = 24 * GIB;

pub const COLLAB_DATA_STORE: ObjectStoreSpec = ObjectStoreSpec {
    bucket: COLLAB_DATA_BUCKET,
    max_age_seconds: Some(DAY),
};
pub const TASTE_DATA_STORE: ObjectStoreSpec = ObjectStoreSpec {
    bucket: TASTE_DATA_BUCKET,
    max_age_seconds: Some(DAY),
};
pub const TASTE_MODELS_STORE: ObjectStoreSpec = ObjectStoreSpec {
    bucket: TASTE_MODELS_BUCKET,
    max_age_seconds: None,
};
pub const WORKER_OBJECT_STORES: [ObjectStoreSpec; 3] =
    [COLLAB_DATA_STORE, TASTE_DATA_STORE, TASTE_MODELS_STORE];

pub const AI_RPC_STREAM: PipelineStreamSpec = work_stream("AI_RPC", &["ai.rpc.>"], 2 * 60, 2 * 60);
pub const INDEX_AUDIO_STREAM: PipelineStreamSpec =
    work_stream("INDEX_AUDIO", &["index.audio.>"], DAY, DAY);
pub const EMBED_LYRICS_STREAM: PipelineStreamSpec =
    work_stream("EMBED_LYRICS", &["embed.lyrics.>"], DAY, DAY);
pub const TRANSCRIBE_STREAM: PipelineStreamSpec =
    work_stream("TRANSCRIBE", &["transcribe.>"], DAY, DAY);
pub const ENCODE_STREAM: PipelineStreamSpec = work_stream("ENCODE", &["encode.>"], DAY, 15 * 60);
pub const TRAIN_COLLAB_STREAM: PipelineStreamSpec =
    work_stream("TRAIN_COLLAB", &["train.collab.>"], 6 * HOUR, HOUR);
pub const TRAIN_TASTE_STREAM: PipelineStreamSpec =
    work_stream("TRAIN_TASTE", &["train.taste.>"], DAY, HOUR);
pub const WORKER_INVALID_STREAM: PipelineStreamSpec = PipelineStreamSpec {
    name: "WORKER_INVALID",
    subjects: &["worker.invalid.>"],
    work_queue: false,
    max_age_seconds: 7 * DAY,
    duplicate_window_seconds: 2 * 60,
    max_bytes: JOB_STREAM_MAX_BYTES,
    discard: StreamDiscard::New,
};
pub const DONE_STREAM: PipelineStreamSpec = PipelineStreamSpec {
    name: "PIPELINE_DONE",
    subjects: &["done.>"],
    work_queue: false,
    max_age_seconds: 3 * DAY,
    duplicate_window_seconds: done_duplicate_window_seconds(),
    max_bytes: DONE_STREAM_MAX_BYTES,
    discard: StreamDiscard::Old,
};
const _: () = assert!(DONE_STREAM.duplicate_window_seconds < DONE_STREAM.max_age_seconds);
pub const STORAGE_EVENTS_STREAM: PipelineStreamSpec = PipelineStreamSpec {
    name: "STORAGE_EVENTS",
    subjects: &["storage.>"],
    work_queue: false,
    max_age_seconds: DAY,
    duplicate_window_seconds: 2 * 60,
    max_bytes: JOB_STREAM_MAX_BYTES,
    discard: StreamDiscard::New,
};

pub const WORKER_STREAMS: [PipelineStreamSpec; 9] = [
    INDEX_AUDIO_STREAM,
    EMBED_LYRICS_STREAM,
    TRANSCRIBE_STREAM,
    ENCODE_STREAM,
    TRAIN_COLLAB_STREAM,
    TRAIN_TASTE_STREAM,
    AI_RPC_STREAM,
    WORKER_INVALID_STREAM,
    DONE_STREAM,
];

pub const PIPELINE_STREAMS: &[PipelineStreamSpec] = &[
    AI_RPC_STREAM,
    INDEX_AUDIO_STREAM,
    EMBED_LYRICS_STREAM,
    TRANSCRIBE_STREAM,
    ENCODE_STREAM,
    TRAIN_COLLAB_STREAM,
    TRAIN_TASTE_STREAM,
    WORKER_INVALID_STREAM,
    DONE_STREAM,
    STORAGE_EVENTS_STREAM,
];

const fn work_stream(
    name: &'static str,
    subjects: &'static [&'static str],
    max_age_seconds: u64,
    duplicate_window_seconds: u64,
) -> PipelineStreamSpec {
    PipelineStreamSpec {
        name,
        subjects,
        work_queue: true,
        max_age_seconds,
        duplicate_window_seconds,
        max_bytes: JOB_STREAM_MAX_BYTES,
        discard: StreamDiscard::New,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn producer(sync_version: Option<&str>) -> Producer {
        Producer {
            worker_id: "gpu-main".to_owned(),
            build: "2026.09.1+abc123".to_owned(),
            models: BTreeMap::from([(
                "mert".to_owned(),
                "OpenMuQ/MuQ-large-msd-iter@a1b2c3d4".to_owned(),
            )]),
            sync_version: sync_version.map(str::to_owned),
        }
    }

    #[test]
    fn an_ok_audio_index_result_carries_generation_attempt_and_producer() {
        let result: AudioIndexResult = serde_json::from_value(serde_json::json!({
            "sc_track_id": "98765",
            "upload_generation": 3,
            "attempt": 1,
            "status": "ok",
            "mert": [0.25],
            "clap": [0.5],
            "fingerprint": "AQADtEmU",
            "producer": {
                "worker_id": "gpu-main",
                "build": "2026.09.1+abc123",
                "models": { "mert": "OpenMuQ/MuQ-large-msd-iter@a1b2c3d4" },
                "sync_version": null
            }
        }))
        .expect("ok audio index result deserializes");

        assert_eq!(result.status, WorkerStatus::Ok);
        assert_eq!((result.upload_generation, result.attempt), (3, 1));
        assert_eq!(result.producer, producer(None));
        assert_eq!(result.reason, None);
    }

    #[test]
    fn a_missing_audio_index_result_has_a_reason_and_no_vectors() {
        let result: AudioIndexResult = serde_json::from_value(serde_json::json!({
            "sc_track_id": "98765",
            "upload_generation": 3,
            "attempt": 2,
            "status": "missing",
            "reason": "audio_not_found",
            "producer": producer(None)
        }))
        .expect("missing audio index result deserializes");

        assert_eq!(result.reason, Some(WorkerReason::AudioNotFound));
        assert_eq!(
            (result.mert, result.clap, result.fingerprint),
            (None, None, None)
        );
    }

    #[test]
    fn an_audio_index_result_without_attempt_or_status_is_refused() {
        let without_attempt = serde_json::from_value::<AudioIndexResult>(serde_json::json!({
            "sc_track_id": "42",
            "upload_generation": 1,
            "status": "ok",
            "producer": producer(None)
        }));
        let without_status = serde_json::from_value::<AudioIndexResult>(serde_json::json!({
            "sc_track_id": "42",
            "upload_generation": 1,
            "attempt": 1,
            "producer": producer(None)
        }));

        assert!(without_attempt.is_err());
        assert!(without_status.is_err());
    }

    #[test]
    fn only_a_canonical_u64_track_id_matches_the_pattern() {
        let pattern = regex::Regex::new(SC_TRACK_ID_PATTERN).expect("the pattern compiles");
        let canonical = |raw: &str| {
            raw.parse::<u64>()
                .is_ok_and(|id| id > 0 && id.to_string() == raw)
        };

        for accepted in ["1", "98765", "9223372036854775807", "9999999999999999999"] {
            assert!(pattern.is_match(accepted), "{accepted}");
            assert!(canonical(accepted), "{accepted}");
        }
        for refused in [
            "0",
            "042",
            "",
            "-1",
            "18446744073709551616",
            "99999999999999999999",
        ] {
            assert!(!pattern.is_match(refused), "{refused}");
        }
    }

    #[test]
    fn a_reason_outside_the_contract_is_refused() {
        let unknown = serde_json::from_value::<EncodeResult>(serde_json::json!({
            "model": "lyrics",
            "hash": "a".repeat(64),
            "status": "failed",
            "reason": "timeout",
            "producer": producer(None),
            "vector": null
        }));

        assert!(unknown.is_err());
    }

    #[test]
    fn an_empty_encode_result_has_a_null_vector() {
        let result = EncodeResult {
            model: EncodeModel::Lyrics,
            hash: "b".repeat(64),
            status: WorkerStatus::Empty,
            reason: Some(WorkerReason::EmptyText),
            detail: None,
            producer: producer(None),
            vector: None,
        };

        assert_eq!(
            serde_json::to_value(result).expect("encode result serializes"),
            serde_json::json!({
                "model": "lyrics",
                "hash": "b".repeat(64),
                "status": "empty",
                "reason": "empty_text",
                "producer": producer(None),
                "vector": null
            })
        );
    }

    #[test]
    fn a_lyrics_embedding_result_echoes_the_request_id_under_vec() {
        let result = LyricsEmbeddingResult {
            sc_track_id: "98765".to_owned(),
            request_id: "lyr:98765:4".to_owned(),
            status: WorkerStatus::Ok,
            reason: None,
            detail: None,
            producer: producer(None),
            vector: Some(vec![0.25, 0.5]),
            language: Some("ru".to_owned()),
        };

        assert_eq!(
            serde_json::to_value(result).expect("lyrics result serializes"),
            serde_json::json!({
                "sc_track_id": "98765",
                "request_id": "lyr:98765:4",
                "status": "ok",
                "producer": producer(None),
                "vec": [0.25, 0.5],
                "language": "ru"
            })
        );
    }

    #[test]
    fn a_rejected_transcription_keeps_its_sync_version_and_metrics() {
        let result: TranscriptionResult = serde_json::from_value(serde_json::json!({
            "sc_track_id": "98765",
            "upload_generation": 3,
            "attempt": 1,
            "mode": "align",
            "status": "rejected",
            "reason": "lyrics_mismatch",
            "detail": "anchor_agreement=0.21",
            "sync_version": "s2.1f3a9c2e.7d1b0e44.9a8b7c6d",
            "confidence": 0.31,
            "placed_share": 0.55,
            "aligned_share": 0.48,
            "lines_total": 20,
            "lines_unplaced": 9,
            "language": "ru",
            "producer": producer(Some("s2.1f3a9c2e.7d1b0e44.9a8b7c6d"))
        }))
        .expect("rejected transcription deserializes");

        assert_eq!(result.reason, Some(WorkerReason::LyricsMismatch));
        assert_eq!(result.sync_version, "s2.1f3a9c2e.7d1b0e44.9a8b7c6d");
        assert_eq!(
            (result.lines_total, result.lines_unplaced),
            (Some(20), Some(9))
        );
        assert_eq!(result.synced_lrc, None);
    }

    #[test]
    fn a_transcription_result_that_is_not_an_alignment_is_refused() {
        let unknown_mode = serde_json::from_value::<TranscriptionResult>(serde_json::json!({
            "sc_track_id": "42",
            "upload_generation": 1,
            "attempt": 1,
            "mode": "full",
            "status": "ok",
            "sync_version": "s2.a.b.c",
            "producer": producer(Some("s2.a.b.c"))
        }));

        assert!(unknown_mode.is_err());
    }

    #[test]
    fn a_transcription_request_always_writes_the_language_key() {
        let request = TranscriptionRequest {
            sc_track_id: "98765".to_owned(),
            upload_generation: 3,
            attempt: 2,
            audio_url: "https://storage.example/audio/98765.opus".to_owned(),
            reference_text: "line".to_owned(),
            reference_lines_total: 1,
            language: None,
            mode: TranscriptionMode::Align,
        };

        let wire = serde_json::to_value(request).expect("transcription request serializes");

        assert_eq!(wire["language"], serde_json::Value::Null);
        assert_eq!(wire["mode"], "align");
    }

    #[test]
    fn collab_vectors_live_next_to_their_input() {
        assert_eq!(
            collab_vectors_object("collab-input-7c1d"),
            "collab-input-7c1d-vectors"
        );
        assert_eq!(collab_vectors_suffix(), "-vectors");
        assert!(
            collab_vectors_suffix()
                .chars()
                .all(|symbol| symbol.is_ascii_alphanumeric() || matches!(symbol, '-' | '_')),
            "the suffix is matched as a literal regex tail"
        );
    }

    #[test]
    fn done_stream_outlives_a_day_of_jobs_downtime_and_drops_the_oldest() {
        assert_eq!(DONE_STREAM.max_age_seconds, 72 * HOUR);
        assert_eq!(DONE_STREAM.duplicate_window_seconds, 12 * HOUR);
        assert_eq!(DONE_STREAM.discard, StreamDiscard::Old);
        assert_eq!(DONE_STREAM.max_bytes, 24 * GIB);
    }

    #[test]
    fn every_other_stream_refuses_new_messages_when_full() {
        for stream in PIPELINE_STREAMS
            .iter()
            .filter(|stream| stream.name != DONE_STREAM.name)
        {
            assert_eq!(stream.discard, StreamDiscard::New, "{}", stream.name);
            assert_eq!(stream.max_bytes, JOB_STREAM_MAX_BYTES, "{}", stream.name);
        }
    }

    #[test]
    fn every_worker_stream_is_provisioned_by_jobs() {
        for stream in WORKER_STREAMS {
            assert!(PIPELINE_STREAMS.contains(&stream), "{}", stream.name);
        }
    }

    #[test]
    fn a_query_point_id_is_stable_and_collision_free() {
        let canonical = "a".repeat(64);
        assert_eq!(
            crate::vector_store::query_point_uuid(&canonical),
            crate::vector_store::query_point_uuid(&canonical)
        );
        assert_ne!(
            crate::vector_store::query_point_uuid(&canonical),
            crate::vector_store::query_point_uuid(&"b".repeat(64))
        );
        assert_eq!(
            crate::vector_store::query_point_uuid("short"),
            crate::vector_store::query_point_uuid("short")
        );
        assert_ne!(
            crate::vector_store::query_point_uuid("short"),
            crate::vector_store::query_point_uuid("other")
        );
    }

    #[test]
    fn storage_rejection_keeps_the_existing_wire_shape() {
        let rejection = StorageTrackRejected {
            sc_track_id: "42".to_owned(),
            reason: StorageRejectionReason::DurationMismatch,
            actual_secs: Some(12.5),
            expected_duration_ms: Some(10_000),
        };

        assert_eq!(
            serde_json::to_value(rejection).expect("storage rejection serializes"),
            serde_json::json!({
                "sc_track_id": "42",
                "reason": "duration_mismatch",
                "actual_secs": 12.5,
                "expected_duration_ms": 10_000
            })
        );
    }

    #[test]
    fn storage_rejection_accepts_legacy_missing_diagnostics() {
        let rejection: StorageTrackRejected = serde_json::from_value(serde_json::json!({
            "sc_track_id": "42"
        }))
        .expect("legacy storage rejection deserializes");

        assert_eq!(rejection.reason, StorageRejectionReason::Unknown);
        assert_eq!(rejection.actual_secs, None);
        assert_eq!(rejection.expected_duration_ms, None);
    }

    #[test]
    fn storage_upload_keeps_real_and_synthetic_wire_shapes() {
        let real: StorageTrackUploaded = serde_json::from_value(serde_json::json!({
            "sc_track_id": "42",
            "storage_url": "https://storage.example/redirect/42.m4a",
            "quality": "hq"
        }))
        .expect("real storage upload deserializes");
        let synthetic: StorageTrackUploaded = serde_json::from_value(serde_json::json!({
            "sc_track_id": "42",
            "storage_url": "https://storage.example/redirect/42.m4a"
        }))
        .expect("synthetic storage upload deserializes");

        assert_eq!(real.quality.as_deref(), Some("hq"));
        assert_eq!(synthetic.quality, None);
    }
}
