use std::collections::BTreeMap;

use schemars::{JsonSchema, Schema, generate::SchemaSettings};
use serde_json::{Map, Value, json};

use crate::pipeline::{
    AI_MATCH_TRACK, AI_RESOLVE_ARTIST, AI_RPC_STREAM, AudioIndexRequest, AudioIndexResult,
    COLLAB_VECTORS_OBJECT_TEMPLATE, CollabTrainRequest, CollabTrainResult, DEADLINE_HEADER,
    DONE_EMBED_LYRICS, DONE_ENCODE, DONE_INDEX_AUDIO, DONE_TRAIN_COLLAB, DONE_TRAIN_TASTE,
    DONE_TRANSCRIBE, EMBED_LYRICS, EMBED_LYRICS_STREAM, ENCODE_STREAM, ENCODE_TEXT_NEW,
    EncodeModel, EncodeRequest, EncodeResult, HEADERS, INDEX_AUDIO, INDEX_AUDIO_STREAM,
    LyricsEmbeddingRequest, LyricsEmbeddingResult, MATCH_TRACK_WINDOW_SECONDS, MAX_MESSAGE_BYTES,
    MatchTrackData, MatchTrackRequest, PipelineStreamSpec, REPLY_TO_HEADER,
    RESOLVE_ARTIST_WINDOW_SECONDS, ResolveArtistData, ResolveArtistRequest, RpcError, TRAIN_COLLAB,
    TRAIN_COLLAB_STREAM, TRAIN_TASTE, TRAIN_TASTE_STREAM, TRANSCRIBE_AUDIO, TRANSCRIBE_STREAM,
    TasteTrainRequest, TasteTrainResult, TranscriptionRequest, TranscriptionResult,
    WORKER_HEALTH_SUBJECT, WORKER_INVALID_SUBJECT, WORKER_OBJECT_STORES, WORKER_STATUS_SUBJECT,
    WORKER_STREAMS, collab_vectors_suffix,
};
use crate::reasons::{FailureClass, WorkerStatus, reasons_in, reasons_of};
use crate::vector_store::{
    QUERY_VEC_MULAN_DIMENSIONS, TRACKS_CLAP_DIMENSIONS, TRACKS_COLLAB_DIMENSIONS,
    TRACKS_LYRICS_DIMENSIONS, TRACKS_MERT_DIMENSIONS, TRACKS_TASTE_DIMENSIONS,
};

pub const CONTRACT_VERSION: u32 = 2;
pub const CONTRACT_PATH: &str = "contract/worker-contract.json";
pub const JSON_SCHEMA_DIALECT: &str = "https://json-schema.org/draft/2020-12/schema";
pub const WORKER_MAX_DELIVER: u64 = 5;
pub const HEARTBEATS_PER_ACK_WAIT: u64 = 5;
pub const BRIDGE_TTL_MARGIN_SECONDS: u64 = 60;
pub const CORRELATION_SEPARATOR: &str = ":";
pub const DONE_MSG_ID_TEMPLATE: &str = "done.{lane}:{correlation}:{task_seq}:{status}";

const HOUR: u64 = 60 * 60;
const TRANSCRIPTION_METRICS: [&str; 6] = [
    "confidence",
    "placed_share",
    "aligned_share",
    "lines_total",
    "lines_unplaced",
    "language",
];

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum WorkerLane {
    Audio,
    Lyrics,
    Transcribe,
    Encode,
    Collab,
    Taste,
    Ai,
}

impl WorkerLane {
    pub const ALL: [Self; 7] = [
        Self::Audio,
        Self::Lyrics,
        Self::Transcribe,
        Self::Encode,
        Self::Collab,
        Self::Taste,
        Self::Ai,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Audio => "audio",
            Self::Lyrics => "lyrics",
            Self::Transcribe => "transcribe",
            Self::Encode => "encode",
            Self::Collab => "collab",
            Self::Taste => "taste",
            Self::Ai => "ai",
        }
    }

    pub const fn spec(self) -> &'static WorkerLaneSpec {
        match self {
            Self::Audio => &AUDIO_LANE,
            Self::Lyrics => &LYRICS_LANE,
            Self::Transcribe => &TRANSCRIBE_LANE,
            Self::Encode => &ENCODE_LANE,
            Self::Collab => &COLLAB_LANE,
            Self::Taste => &TASTE_LANE,
            Self::Ai => &AI_LANE,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NakBackoff {
    pub base_s: u64,
    pub cap_s: u64,
}

impl NakBackoff {
    pub const fn delay_s(self, num_delivered: u64) -> u64 {
        let exponent = num_delivered.saturating_sub(1);
        let factor = if exponent >= u64::BITS as u64 - 1 {
            u64::MAX
        } else {
            1 << exponent
        };
        let delay = self.base_s.saturating_mul(factor);
        if delay < self.cap_s {
            delay
        } else {
            self.cap_s
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WorkerLaneSpec {
    pub lane: WorkerLane,
    pub stream: PipelineStreamSpec,
    pub durable: &'static str,
    pub filter_subject: &'static str,
    pub done_subject: Option<&'static str>,
    pub correlation_prefix: &'static str,
    pub correlation: &'static [&'static str],
    pub echo: &'static [(&'static str, &'static str)],
    pub deadline_s: u64,
    pub ack_wait_s: u64,
    pub max_ack_pending: u64,
    pub nak: Option<NakBackoff>,
    pub result_max_bytes: u64,
    pub public: bool,
}

impl WorkerLaneSpec {
    pub const fn max_deliver(&self) -> u64 {
        WORKER_MAX_DELIVER
    }

    pub const fn heartbeat_s(&self) -> u64 {
        self.ack_wait_s / HEARTBEATS_PER_ACK_WAIT
    }

    pub const fn is_rpc(&self) -> bool {
        self.done_subject.is_none()
    }

    pub const fn delivery_s(&self) -> u64 {
        match self.bridge_ttl_s() {
            Some(ttl) if ttl > self.deadline_s => ttl,
            _ => self.deadline_s,
        }
    }

    pub const fn attempt_window_s(&self) -> Option<u64> {
        let Some(nak) = self.nak else {
            return None;
        };
        let mut window = self.max_deliver() * self.delivery_s();
        let mut delivery = 1;
        while delivery < self.max_deliver() {
            window += nak.delay_s(delivery);
            delivery += 1;
        }
        Some(window)
    }

    pub const fn quarantine_after_s(&self) -> Option<u64> {
        match self.attempt_window_s() {
            Some(window) => Some(self.stream.max_age_seconds + window),
            None => None,
        }
    }

    pub const fn bridge_ttl_s(&self) -> Option<u64> {
        if self.public {
            Some(self.deadline_s + BRIDGE_TTL_MARGIN_SECONDS)
        } else {
            None
        }
    }

    pub fn done_msg_id(&self, correlation: &str, task_seq: u64, status: WorkerStatus) -> String {
        DONE_MSG_ID_TEMPLATE
            .replace("{lane}", self.lane.as_str())
            .replace("{correlation}", correlation)
            .replace("{task_seq}", &task_seq.to_string())
            .replace("{status}", status.as_str())
    }
}

pub const AUDIO_LANE: WorkerLaneSpec = WorkerLaneSpec {
    lane: WorkerLane::Audio,
    stream: INDEX_AUDIO_STREAM,
    durable: "audio-workers",
    filter_subject: INDEX_AUDIO,
    done_subject: Some(DONE_INDEX_AUDIO),
    correlation_prefix: "index_audio",
    correlation: &["sc_track_id", "upload_generation", "attempt"],
    echo: &[
        ("sc_track_id", "sc_track_id"),
        ("upload_generation", "upload_generation"),
        ("attempt", "attempt"),
    ],
    deadline_s: 180,
    ack_wait_s: 60,
    max_ack_pending: 256,
    nak: Some(NakBackoff {
        base_s: 30,
        cap_s: 600,
    }),
    result_max_bytes: 64 * 1024,
    public: true,
};

pub const LYRICS_LANE: WorkerLaneSpec = WorkerLaneSpec {
    lane: WorkerLane::Lyrics,
    stream: EMBED_LYRICS_STREAM,
    durable: "lyrics-workers",
    filter_subject: EMBED_LYRICS,
    done_subject: Some(DONE_EMBED_LYRICS),
    correlation_prefix: "embed_lyrics",
    correlation: &["sc_track_id", "request_id"],
    echo: &[("sc_track_id", "sc_track_id"), ("request_id", "request_id")],
    deadline_s: 60,
    ack_wait_s: 30,
    max_ack_pending: 512,
    nak: Some(NakBackoff {
        base_s: 15,
        cap_s: 300,
    }),
    result_max_bytes: 32 * 1024,
    public: true,
};

pub const TRANSCRIBE_LANE: WorkerLaneSpec = WorkerLaneSpec {
    lane: WorkerLane::Transcribe,
    stream: TRANSCRIBE_STREAM,
    durable: "transcribe-workers",
    filter_subject: TRANSCRIBE_AUDIO,
    done_subject: Some(DONE_TRANSCRIBE),
    correlation_prefix: "transcribe",
    correlation: &["sc_track_id", "upload_generation", "attempt"],
    echo: &[
        ("sc_track_id", "sc_track_id"),
        ("upload_generation", "upload_generation"),
        ("attempt", "attempt"),
        ("mode", "mode"),
    ],
    deadline_s: 900,
    ack_wait_s: 120,
    max_ack_pending: 64,
    nak: Some(NakBackoff {
        base_s: 60,
        cap_s: 900,
    }),
    result_max_bytes: 512 * 1024,
    public: true,
};

pub const ENCODE_LANE: WorkerLaneSpec = WorkerLaneSpec {
    lane: WorkerLane::Encode,
    stream: ENCODE_STREAM,
    durable: "encode-workers",
    filter_subject: ENCODE_TEXT_NEW,
    done_subject: Some(DONE_ENCODE),
    correlation_prefix: "encode",
    correlation: &["model", "hash"],
    echo: &[("model", "model"), ("hash", "hash")],
    deadline_s: 30,
    ack_wait_s: 30,
    max_ack_pending: 256,
    nak: Some(NakBackoff {
        base_s: 5,
        cap_s: 60,
    }),
    result_max_bytes: 16 * 1024,
    public: false,
};

pub const COLLAB_LANE: WorkerLaneSpec = WorkerLaneSpec {
    lane: WorkerLane::Collab,
    stream: TRAIN_COLLAB_STREAM,
    durable: "collab-workers",
    filter_subject: TRAIN_COLLAB,
    done_subject: Some(DONE_TRAIN_COLLAB),
    correlation_prefix: "train_collab",
    correlation: &["object"],
    echo: &[("object", "input_object")],
    deadline_s: 3600,
    ack_wait_s: 300,
    max_ack_pending: 1,
    nak: Some(NakBackoff {
        base_s: 120,
        cap_s: 1800,
    }),
    result_max_bytes: 8 * 1024,
    public: false,
};

pub const TASTE_LANE: WorkerLaneSpec = WorkerLaneSpec {
    lane: WorkerLane::Taste,
    stream: TRAIN_TASTE_STREAM,
    durable: "taste-workers",
    filter_subject: TRAIN_TASTE,
    done_subject: Some(DONE_TRAIN_TASTE),
    correlation_prefix: "train_taste",
    correlation: &["object"],
    echo: &[("object", "input_object")],
    deadline_s: 7200,
    ack_wait_s: 300,
    max_ack_pending: 1,
    nak: Some(NakBackoff {
        base_s: 300,
        cap_s: 1800,
    }),
    result_max_bytes: 8 * 1024,
    public: false,
};

pub const AI_LANE: WorkerLaneSpec = WorkerLaneSpec {
    lane: WorkerLane::Ai,
    stream: AI_RPC_STREAM,
    durable: "ai-workers",
    filter_subject: "ai.rpc.>",
    done_subject: None,
    correlation_prefix: "ai",
    correlation: &[],
    echo: &[],
    deadline_s: widest_rpc_window_seconds(),
    ack_wait_s: 30,
    max_ack_pending: 128,
    nak: None,
    result_max_bytes: 16 * 1024,
    public: false,
};

pub const WORKER_LANES: [WorkerLaneSpec; 7] = [
    AUDIO_LANE,
    LYRICS_LANE,
    TRANSCRIBE_LANE,
    ENCODE_LANE,
    COLLAB_LANE,
    TASTE_LANE,
    AI_LANE,
];

pub const fn done_duplicate_window_seconds() -> u64 {
    let mut widest = 0;
    let mut index = 0;
    while index < WORKER_LANES.len() {
        if let Some(window) = WORKER_LANES[index].attempt_window_s()
            && window > widest
        {
            widest = window;
        }
        index += 1;
    }
    widest.div_ceil(HOUR) * HOUR
}

const fn widest_rpc_window_seconds() -> u64 {
    if RESOLVE_ARTIST_WINDOW_SECONDS > MATCH_TRACK_WINDOW_SECONDS {
        RESOLVE_ARTIST_WINDOW_SECONDS
    } else {
        MATCH_TRACK_WINDOW_SECONDS
    }
}

pub fn render_worker_contract() -> String {
    let mut rendered =
        serde_json::to_string_pretty(&worker_contract()).expect("the worker contract serializes");
    rendered.push('\n');
    rendered
}

pub fn worker_contract() -> Value {
    json!({
        "version": CONTRACT_VERSION,
        "streams": streams(),
        "object_stores": object_stores(),
        "lanes": lanes(),
        "dimensions": {
            "mert": TRACKS_MERT_DIMENSIONS,
            "clap": TRACKS_CLAP_DIMENSIONS,
            "lyrics": TRACKS_LYRICS_DIMENSIONS,
            "mulan_text": QUERY_VEC_MULAN_DIMENSIONS,
            "collab": TRACKS_COLLAB_DIMENSIONS,
            "taste": TRACKS_TASTE_DIMENSIONS,
        },
        "schemas": schemas(),
        "reasons": reasons(),
        "reason_classes": reason_classes(),
        "headers": headers(),
        "rpc": {
            "reply_header": REPLY_TO_HEADER,
            "deadline_header": DEADLINE_HEADER,
            "windows_s": {
                "resolve_artist": RESOLVE_ARTIST_WINDOW_SECONDS,
                "match_track": MATCH_TRACK_WINDOW_SECONDS,
            },
        },
        "subjects": {
            "invalid": WORKER_INVALID_SUBJECT,
            "status": WORKER_STATUS_SUBJECT,
            "health": WORKER_HEALTH_SUBJECT,
        },
        "limits": { "max_message_bytes": MAX_MESSAGE_BYTES },
    })
}

fn headers() -> Map<String, Value> {
    HEADERS
        .iter()
        .map(|(role, name)| ((*role).to_owned(), Value::from(*name)))
        .collect()
}

fn streams() -> Map<String, Value> {
    WORKER_STREAMS
        .iter()
        .map(|stream| {
            let spec = json!({
                "subjects": stream.subjects,
                "retention": if stream.work_queue { "work_queue" } else { "limits" },
                "max_age_s": stream.max_age_seconds,
                "duplicate_window_s": stream.duplicate_window_seconds,
                "max_bytes": stream.max_bytes,
                "discard": stream.discard.as_str(),
            });
            (stream.name.to_owned(), spec)
        })
        .collect()
}

fn object_stores() -> Map<String, Value> {
    WORKER_OBJECT_STORES
        .iter()
        .map(|store| {
            let spec = json!({ "max_age_s": store.max_age_seconds });
            (store.bucket.to_owned(), spec)
        })
        .collect()
}

fn lanes() -> Map<String, Value> {
    WORKER_LANES
        .iter()
        .map(|spec| (spec.lane.as_str().to_owned(), lane(spec)))
        .collect()
}

fn lane(spec: &WorkerLaneSpec) -> Value {
    let echo: Map<String, Value> = spec
        .echo
        .iter()
        .map(|(request, done)| ((*request).to_owned(), Value::from(*done)))
        .collect();
    let reopenable: Vec<&str> = spec
        .lane
        .reopenable()
        .iter()
        .map(|reason| reason.as_str())
        .collect();
    let reasons: Vec<&str> = spec
        .lane
        .reasons()
        .iter()
        .map(|reason| reason.as_str())
        .collect();
    let worker_reasons: Vec<&str> = spec
        .lane
        .worker_reasons()
        .map(|reason| reason.as_str())
        .collect();
    let result_object_template =
        (spec.lane == WorkerLane::Collab).then_some(COLLAB_VECTORS_OBJECT_TEMPLATE);
    json!({
        "stream": spec.stream.name,
        "durable": spec.durable,
        "filter_subject": spec.filter_subject,
        "done_subject": spec.done_subject,
        "done_msg_id_template": spec.done_subject.map(|_| DONE_MSG_ID_TEMPLATE),
        "correlation_prefix": spec.correlation_prefix,
        "correlation_separator": CORRELATION_SEPARATOR,
        "correlation": spec.correlation,
        "echo": echo,
        "deadline_s": spec.deadline_s,
        "ack_wait_s": spec.ack_wait_s,
        "heartbeat_s": spec.heartbeat_s(),
        "max_deliver": spec.max_deliver(),
        "max_ack_pending": spec.max_ack_pending,
        "nak_base_s": spec.nak.map(|nak| nak.base_s),
        "nak_cap_s": spec.nak.map(|nak| nak.cap_s),
        "attempt_window_s": spec.attempt_window_s(),
        "quarantine_after_s": spec.quarantine_after_s(),
        "bridge_ttl_s": spec.bridge_ttl_s(),
        "result_max_bytes": spec.result_max_bytes,
        "result_object_template": result_object_template,
        "reasons": reasons,
        "worker_reasons": worker_reasons,
        "reopenable": reopenable,
        "rpc": spec.is_rpc(),
        "public": spec.public,
    })
}

fn reasons() -> Map<String, Value> {
    WorkerStatus::ALL
        .into_iter()
        .map(|status| {
            let names: Vec<&str> = reasons_of(status).map(|reason| reason.as_str()).collect();
            (status.as_str().to_owned(), json!(names))
        })
        .collect()
}

fn reason_classes() -> Map<String, Value> {
    FailureClass::ALL
        .into_iter()
        .map(|class| {
            let names: Vec<&str> = reasons_in(class).map(|reason| reason.as_str()).collect();
            (class.as_str().to_owned(), json!(names))
        })
        .collect()
}

pub fn schemas() -> BTreeMap<String, Value> {
    BTreeMap::from([
        request::<AudioIndexRequest>(INDEX_AUDIO),
        done::<AudioIndexResult>(WorkerLane::Audio),
        request::<LyricsEmbeddingRequest>(EMBED_LYRICS),
        done::<LyricsEmbeddingResult>(WorkerLane::Lyrics),
        request::<TranscriptionRequest>(TRANSCRIBE_AUDIO),
        done::<TranscriptionResult>(WorkerLane::Transcribe),
        request::<EncodeRequest>(ENCODE_TEXT_NEW),
        done::<EncodeResult>(WorkerLane::Encode),
        request::<CollabTrainRequest>(TRAIN_COLLAB),
        done::<CollabTrainResult>(WorkerLane::Collab),
        request::<TasteTrainRequest>(TRAIN_TASTE),
        done::<TasteTrainResult>(WorkerLane::Taste),
        request::<ResolveArtistRequest>(AI_RESOLVE_ARTIST),
        request::<MatchTrackRequest>(AI_MATCH_TRACK),
        rpc_reply::<ResolveArtistData>(AI_RESOLVE_ARTIST),
        rpc_reply::<MatchTrackData>(AI_MATCH_TRACK),
    ])
}

pub fn rpc_reply_subject(method: &str) -> String {
    format!("{method}.reply")
}

fn request<T: JsonSchema>(subject: &str) -> (String, Value) {
    (subject.to_owned(), root_schema::<T>(subject).to_value())
}

fn done<T: JsonSchema>(lane: WorkerLane) -> (String, Value) {
    let subject = lane
        .spec()
        .done_subject
        .expect("only a lane with a done subject has a done schema");
    let mut schema = root_schema::<T>(subject);
    let rules: Vec<Value> = WorkerStatus::ALL
        .into_iter()
        .map(|status| {
            json!({
                "if": { "properties": { "status": { "const": status.as_str() } } },
                "then": status_rule(lane, status),
            })
        })
        .collect();
    schema.insert("allOf".to_owned(), Value::Array(rules));
    (subject.to_owned(), schema.to_value())
}

fn rpc_reply<T: JsonSchema>(method: &str) -> (String, Value) {
    let subject = rpc_reply_subject(method);
    let mut generator = SchemaSettings::draft2020_12().into_generator();
    let data = generator.subschema_for::<T>();
    let error = generator.subschema_for::<RpcError>();
    let schema = json!({
        "$schema": JSON_SCHEMA_DIALECT,
        "title": subject,
        "oneOf": [
            {
                "type": "object",
                "required": ["ok", "data"],
                "properties": { "ok": { "const": true }, "data": data },
            },
            {
                "type": "object",
                "required": ["ok", "error"],
                "properties": { "ok": { "const": false }, "error": error },
            },
        ],
        "$defs": generator.take_definitions(true),
    });
    (subject, schema)
}

fn root_schema<T: JsonSchema>(title: &str) -> Schema {
    let mut schema = SchemaSettings::draft2020_12()
        .into_generator()
        .into_root_schema_for::<T>();
    schema.insert("title".to_owned(), Value::from(title));
    schema
}

fn status_rule(lane: WorkerLane, status: WorkerStatus) -> Value {
    let mut rule = Map::new();
    let mut required: Vec<&str> = Vec::new();
    let mut properties = properties_on(lane, status);
    if status == WorkerStatus::Ok {
        rule.insert("not".to_owned(), json!({ "required": ["reason"] }));
    } else {
        let allowed: Vec<&str> = reasons_of(status)
            .filter(|reason| reason.is_published_on(lane))
            .map(|reason| reason.as_str())
            .collect();
        if allowed.is_empty() {
            return Value::Bool(false);
        }
        required.push("reason");
        properties.insert("reason".to_owned(), json!({ "enum": allowed }));
    }
    required.extend(required_on(lane, status));
    if !required.is_empty() {
        rule.insert("required".to_owned(), json!(required));
    }
    if !properties.is_empty() {
        rule.insert("properties".to_owned(), Value::Object(properties));
    }
    if lane == WorkerLane::Encode && status == WorkerStatus::Ok {
        rule.insert("allOf".to_owned(), json!([encode_vector_by_model()]));
    }
    Value::Object(rule)
}

fn required_on(lane: WorkerLane, status: WorkerStatus) -> Vec<&'static str> {
    match (lane, status) {
        (WorkerLane::Audio, WorkerStatus::Ok) => vec!["mert", "clap"],
        (WorkerLane::Lyrics, WorkerStatus::Ok) => vec!["vec", "language"],
        (WorkerLane::Lyrics, WorkerStatus::Empty) => vec!["language"],
        (WorkerLane::Transcribe, WorkerStatus::Ok) => {
            let mut fields = TRANSCRIPTION_METRICS.to_vec();
            fields.push("synced_lrc");
            fields
        }
        (WorkerLane::Transcribe, WorkerStatus::Rejected) => TRANSCRIPTION_METRICS.to_vec(),
        (WorkerLane::Collab, WorkerStatus::Ok) => vec!["object"],
        (WorkerLane::Taste, WorkerStatus::Ok) => {
            vec!["version", "object", "items_count", "users_count", "metrics"]
        }
        _ => Vec::new(),
    }
}

fn properties_on(lane: WorkerLane, status: WorkerStatus) -> Map<String, Value> {
    let succeeded = status == WorkerStatus::Ok;
    let mut properties = Map::new();
    match lane {
        WorkerLane::Encode if !succeeded => {
            properties.insert("vector".to_owned(), json!({ "type": "null" }));
        }
        WorkerLane::Collab => {
            properties.insert("trained".to_owned(), json!({ "const": succeeded }));
            if succeeded {
                let pattern = format!("{}$", collab_vectors_suffix());
                properties.insert("object".to_owned(), json!({ "pattern": pattern }));
            }
        }
        _ => {}
    }
    properties
}

fn encode_vector_by_model() -> Value {
    json!({
        "if": { "properties": { "model": { "const": EncodeModel::Mulan.as_str() } } },
        "then": { "properties": { "vector": vector(EncodeModel::Mulan.dimensions()) } },
        "else": { "properties": { "vector": vector(EncodeModel::Lyrics.dimensions()) } },
    })
}

fn vector(dimensions: u64) -> Value {
    json!({
        "type": "array",
        "items": { "type": "number" },
        "minItems": dimensions,
        "maxItems": dimensions,
    })
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::pipeline::DONE_STREAM;
    use crate::reasons::WorkerReason;

    fn contract() -> Value {
        worker_contract()
    }

    #[test]
    fn the_exported_file_is_what_the_crate_generates() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(CONTRACT_PATH);
        let on_disk = std::fs::read_to_string(&path).unwrap_or_default();

        assert!(
            on_disk == render_worker_contract(),
            "{} drifted from backend-contracts; regenerate it with `cargo run --bin export-worker-contract`",
            path.display()
        );
    }

    #[test]
    fn lane_windows_follow_the_formulas_of_the_design() {
        let table = [
            (WorkerLane::Audio, 12, 1650, 88_050, Some(240)),
            (WorkerLane::Lyrics, 6, 825, 87_225, Some(120)),
            (WorkerLane::Transcribe, 24, 5700, 92_100, Some(960)),
            (WorkerLane::Encode, 6, 225, 86_625, None),
            (WorkerLane::Collab, 60, 19_800, 41_400, None),
            (WorkerLane::Taste, 60, 39_900, 126_300, None),
        ];
        for (lane, heartbeat, window, quarantine, bridge) in table {
            let spec = lane.spec();
            let computed = (
                spec.heartbeat_s(),
                spec.attempt_window_s(),
                spec.quarantine_after_s(),
                spec.bridge_ttl_s(),
            );
            let expected = (heartbeat, Some(window), Some(quarantine), bridge);
            assert_eq!(computed, expected, "{}", lane.as_str());
        }
        assert_eq!(AI_LANE.deadline_s, 20);
        assert_eq!(AI_LANE.attempt_window_s(), None);
    }

    #[test]
    fn a_public_delivery_is_held_for_the_whole_bridge_ttl() {
        for spec in WORKER_LANES.iter().filter(|spec| spec.nak.is_some()) {
            let nak = spec.nak.expect("filtered by nak");
            let delays: u64 = (1..spec.max_deliver()).map(|n| nak.delay_s(n)).sum();
            let delivery = spec.bridge_ttl_s().unwrap_or(spec.deadline_s);
            let lane = spec.lane.as_str();

            assert!(delivery >= spec.deadline_s, "{lane}");
            assert_eq!(spec.delivery_s(), delivery, "{lane}");
            assert_eq!(
                spec.attempt_window_s(),
                Some(spec.max_deliver() * delivery + delays),
                "{lane}"
            );
        }
        let transcribe_nak = TRANSCRIBE_LANE.nak.expect("transcribe naks");
        let all_bridged = 5 * 960 + (1..5).map(|n| transcribe_nak.delay_s(n)).sum::<u64>();
        assert_eq!(TRANSCRIBE_LANE.attempt_window_s(), Some(all_bridged));
    }

    #[test]
    fn nak_delay_doubles_up_to_the_cap_and_is_never_zero() {
        let nak = TRANSCRIBE_LANE.nak.expect("transcribe naks");
        let delays: Vec<u64> = (1..=6).map(|delivery| nak.delay_s(delivery)).collect();

        assert_eq!(delays, [60, 120, 240, 480, 900, 900]);
        assert_eq!(nak.delay_s(0), 60);
        assert_eq!(nak.delay_s(u64::MAX), 900);
    }

    #[test]
    fn the_done_duplicate_window_covers_the_widest_attempt_window() {
        assert_eq!(done_duplicate_window_seconds(), 12 * HOUR);
        for spec in WORKER_LANES {
            assert!(spec.attempt_window_s().unwrap_or(0) <= DONE_STREAM.duplicate_window_seconds);
        }
    }

    #[test]
    fn a_lane_that_can_be_reopened_correlates_by_attempt_request_or_object() {
        for spec in WORKER_LANES
            .iter()
            .filter(|spec| !spec.lane.reopenable().is_empty())
        {
            assert!(
                spec.correlation
                    .iter()
                    .any(|field| matches!(*field, "attempt" | "request_id" | "object")),
                "{}",
                spec.lane.as_str()
            );
        }
    }

    #[test]
    fn every_lane_filter_lives_in_its_stream_and_every_limit_fits_a_message() {
        for spec in WORKER_LANES {
            let prefix = spec.stream.subjects[0].trim_end_matches('>');
            let lane = spec.lane.as_str();
            assert!(spec.filter_subject.starts_with(prefix), "{lane}");
            assert!(spec.result_max_bytes <= u64::from(MAX_MESSAGE_BYTES));
            assert!(spec.heartbeat_s() * 2 < spec.ack_wait_s);
        }
    }

    #[test]
    fn only_lanes_of_the_design_reach_public_nodes() {
        let public: Vec<&str> = WORKER_LANES
            .iter()
            .filter(|spec| spec.public)
            .map(|spec| spec.lane.as_str())
            .collect();

        assert_eq!(public, ["audio", "lyrics", "transcribe"]);
    }

    #[test]
    fn dimensions_come_from_the_vector_store_constants() {
        assert_eq!(
            contract()["dimensions"],
            json!({
                "mert": 1024,
                "clap": 512,
                "lyrics": 1024,
                "mulan_text": 512,
                "collab": 128,
                "taste": 128
            })
        );
    }

    #[test]
    fn every_task_and_done_subject_has_a_schema() {
        let schemas = schemas();
        for spec in WORKER_LANES.iter().filter(|spec| !spec.is_rpc()) {
            assert!(schemas.contains_key(spec.filter_subject));
            assert!(schemas.contains_key(spec.done_subject.unwrap_or_default()));
        }
        for method in [AI_RESOLVE_ARTIST, AI_MATCH_TRACK] {
            assert!(schemas.contains_key(method));
            assert!(schemas.contains_key(&rpc_reply_subject(method)));
        }
        for schema in schemas.values() {
            assert_eq!(schema["$schema"], JSON_SCHEMA_DIALECT);
        }
    }

    #[test]
    fn a_failed_done_accepts_only_the_reopenable_reasons_of_its_lane() {
        let schemas = schemas();
        let failed_reasons = |subject: &str| {
            let failed = schemas[subject]["allOf"]
                .as_array()
                .expect("done schema has status rules")
                .iter()
                .find(|rule| rule["if"]["properties"]["status"]["const"] == "failed")
                .expect("failed rule");
            names(&failed["then"]["properties"]["reason"]["enum"])
        };

        let lyrics = failed_reasons(DONE_EMBED_LYRICS);
        let audio = failed_reasons(DONE_INDEX_AUDIO);
        let encode = failed_reasons(DONE_ENCODE);

        assert!(!lyrics.contains(&"public_node_timeout"));
        assert!(audio.contains(&"public_node_timeout"));
        assert!(!encode.contains(&"engine_restarted"));
        assert!(encode.contains(&"hash_mismatch"));
    }

    fn rule_of<'a>(schemas: &'a BTreeMap<String, Value>, subject: &str, status: &str) -> &'a Value {
        let rules = schemas[subject]["allOf"]
            .as_array()
            .expect("done schema has status rules");
        &rules
            .iter()
            .find(|rule| rule["if"]["properties"]["status"]["const"] == status)
            .expect("a rule per status")["then"]
    }

    #[test]
    fn a_done_schema_accepts_only_the_reasons_of_its_lane() {
        let schemas = schemas();
        let reasons = |subject: &str, status: &str| {
            names(&rule_of(&schemas, subject, status)["properties"]["reason"]["enum"])
        };

        assert_eq!(rule_of(&schemas, DONE_ENCODE, "rejected"), &json!(false));
        assert_eq!(
            rule_of(&schemas, DONE_INDEX_AUDIO, "rejected"),
            &json!(false)
        );
        assert_eq!(
            rule_of(&schemas, DONE_EMBED_LYRICS, "missing"),
            &json!(false)
        );
        assert_eq!(
            reasons(DONE_INDEX_AUDIO, "empty"),
            ["silent_audio", "audio_too_short"]
        );
        assert_eq!(reasons(DONE_ENCODE, "empty"), ["empty_text"]);
        assert!(!reasons(DONE_TRANSCRIBE, "rejected").contains(&"below_baseline"));
        assert_eq!(reasons(DONE_TRAIN_COLLAB, "rejected"), ["below_baseline"]);
        assert!(!reasons(DONE_EMBED_LYRICS, "failed").contains(&"download_failed"));
        let lanes = contract()["lanes"].clone();
        for spec in WORKER_LANES.iter().filter(|spec| !spec.is_rpc()) {
            let subject = spec.done_subject.unwrap_or_default();
            let exported = names(&lanes[spec.lane.as_str()]["reasons"]);
            let mut accepted: Vec<&str> = WorkerStatus::ALL
                .into_iter()
                .filter(|status| *status != WorkerStatus::Ok)
                .flat_map(|status| reasons(subject, status.as_str()))
                .collect();
            accepted.sort_by_key(|name| exported.iter().position(|known| known == name));
            assert_eq!(accepted, exported, "{}", spec.lane.as_str());
        }
    }

    #[test]
    fn the_done_msg_id_is_rendered_from_the_exported_template() {
        let exported = contract();
        let audio = &exported["lanes"]["audio"];
        let template = audio["done_msg_id_template"].as_str().unwrap_or_default();
        let separator = audio["correlation_separator"].as_str().unwrap_or_default();
        let correlation = ["index_audio", "98765", "3", "1"].join(separator);
        let rendered = template
            .replace("{lane}", "audio")
            .replace("{correlation}", &correlation)
            .replace("{task_seq}", "77")
            .replace("{status}", "ok");

        assert_eq!(rendered, "done.audio:index_audio:98765:3:1:77:ok");
        assert_eq!(
            AUDIO_LANE.done_msg_id(&correlation, 77, WorkerStatus::Ok),
            rendered
        );
        assert_eq!(exported["lanes"]["ai"]["done_msg_id_template"], Value::Null);
        for spec in WORKER_LANES {
            let lane = &exported["lanes"][spec.lane.as_str()];
            assert_eq!(lane["correlation_separator"], CORRELATION_SEPARATOR);
        }
    }

    #[test]
    fn every_rule_of_a_done_schema_is_keyed_by_one_status() {
        let schemas = schemas();
        for spec in WORKER_LANES.iter().filter(|spec| !spec.is_rpc()) {
            let rules = schemas[spec.done_subject.unwrap_or_default()]["allOf"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            let statuses: Vec<&str> = rules
                .iter()
                .filter_map(|rule| rule["if"]["properties"]["status"]["const"].as_str())
                .collect();

            assert_eq!(statuses, ["ok", "empty", "missing", "rejected", "failed"]);
            assert_eq!(statuses.len(), rules.len());
        }
    }

    #[test]
    fn a_key_that_is_always_sent_may_still_be_null() {
        let schemas = schemas();
        let request = &schemas[TRANSCRIBE_AUDIO];
        let producer = &schemas[DONE_INDEX_AUDIO]["$defs"]["Producer"];
        let nullable = json!(["string", "null"]);

        assert!(names(&request["required"]).contains(&"language"));
        assert_eq!(request["properties"]["language"]["type"], nullable);
        assert!(names(&producer["required"]).contains(&"sync_version"));
        assert_eq!(producer["properties"]["sync_version"]["type"], nullable);
    }

    fn names(list: &Value) -> Vec<&str> {
        list.as_array()
            .map(|items| items.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default()
    }

    #[test]
    fn text_limits_are_counted_in_utf8_bytes() {
        let schemas = schemas();
        let text = &schemas[TRANSCRIBE_AUDIO]["properties"]["reference_text"];

        assert_eq!(text[crate::pipeline::MAX_UTF8_BYTES_KEYWORD], 16_000);
        assert_eq!(
            schemas[ENCODE_TEXT_NEW]["properties"]["text"][crate::pipeline::MAX_UTF8_BYTES_KEYWORD],
            512
        );
    }

    #[test]
    fn the_collab_vectors_object_is_exported_and_enforced_by_the_done_schema() {
        let exported = contract();
        let template = exported["lanes"]["collab"]["result_object_template"]
            .as_str()
            .unwrap_or_default();
        let rendered = template.replace(crate::pipeline::COLLAB_OBJECT_PLACEHOLDER, "in-7c1d");

        assert_eq!(rendered, crate::pipeline::collab_vectors_object("in-7c1d"));
        assert_eq!(
            exported["lanes"]["audio"]["result_object_template"],
            Value::Null
        );
        let schemas = schemas();
        let ok = rule_of(&schemas, DONE_TRAIN_COLLAB, "ok");
        assert_eq!(
            ok["properties"]["object"]["pattern"],
            format!("{}$", collab_vectors_suffix())
        );
        let failed = rule_of(&schemas, DONE_TRAIN_COLLAB, "failed");
        assert_eq!(failed["properties"]["object"], Value::Null);
    }

    #[test]
    fn every_header_is_exported_under_its_role() {
        let exported = contract()["headers"].clone();
        let expected = json!({
            "msg_id": crate::pipeline::MSG_ID_HEADER,
            "reply_to": REPLY_TO_HEADER,
            "deadline": DEADLINE_HEADER,
            "worker_id": crate::pipeline::WORKER_ID_HEADER,
            "worker_build": crate::pipeline::WORKER_BUILD_HEADER,
            "deliveries": crate::pipeline::DELIVERIES_HEADER,
            "public_node": crate::pipeline::PUBLIC_NODE_HEADER,
        });

        assert_eq!(exported, expected);
        assert_eq!(contract()["rpc"]["reply_header"], exported["reply_to"]);
        assert_eq!(contract()["rpc"]["deadline_header"], exported["deadline"]);
    }

    #[test]
    fn a_worker_may_publish_every_lane_reason_except_those_the_bus_sets() {
        let lanes = contract()["lanes"].clone();
        for spec in WORKER_LANES {
            let lane = &lanes[spec.lane.as_str()];
            let all = names(&lane["reasons"]);
            let published = names(&lane["worker_reasons"]);
            let expected: Vec<&str> = all
                .iter()
                .copied()
                .filter(|name| !matches!(*name, "worker_lost" | "public_node_timeout"))
                .collect();

            assert_eq!(published, expected, "{}", spec.lane.as_str());
        }
        assert!(names(&lanes["audio"]["reasons"]).contains(&"public_node_timeout"));
        assert!(!names(&lanes["audio"]["worker_reasons"]).contains(&"worker_lost"));
        assert!(
            WorkerReason::SET_BY_BUS
                .iter()
                .all(|reason| reason.status() == WorkerStatus::Failed)
        );
    }

    #[test]
    fn the_reason_enum_lists_every_reason_once() {
        let listed: Vec<Value> = contract()["reasons"]
            .as_object()
            .expect("reasons by status")
            .values()
            .flat_map(|names| names.as_array().cloned().unwrap_or_default())
            .collect();

        assert_eq!(listed.len(), WorkerReason::ALL.len());
        assert_eq!(contract()["reasons"]["ok"], json!([]));
    }
}
