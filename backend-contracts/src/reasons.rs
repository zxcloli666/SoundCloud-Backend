use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::worker_contract::WorkerLane;

#[derive(
    Clone, Copy, Debug, Deserialize, Eq, Hash, JsonSchema, Ord, PartialEq, PartialOrd, Serialize,
)]
#[serde(rename_all = "snake_case")]
pub enum WorkerStatus {
    Ok,
    Empty,
    Missing,
    Rejected,
    Failed,
}

impl WorkerStatus {
    pub const ALL: [Self; 5] = [
        Self::Ok,
        Self::Empty,
        Self::Missing,
        Self::Rejected,
        Self::Failed,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Empty => "empty",
            Self::Missing => "missing",
            Self::Rejected => "rejected",
            Self::Failed => "failed",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum FailureClass {
    Deterministic,
    Transient,
    Reopenable,
}

impl FailureClass {
    pub const ALL: [Self; 3] = [Self::Deterministic, Self::Transient, Self::Reopenable];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Deterministic => "deterministic",
            Self::Transient => "transient",
            Self::Reopenable => "reopenable",
        }
    }
}

#[derive(
    Clone, Copy, Debug, Deserialize, Eq, Hash, JsonSchema, Ord, PartialEq, PartialOrd, Serialize,
)]
#[serde(rename_all = "snake_case")]
pub enum WorkerReason {
    EmptyText,
    EmptyReferenceText,
    SilentAudio,
    AudioTooShort,
    EmptyVocab,
    TooFewUsers,
    AudioNotFound,
    AudioForbidden,
    ObjectNotFound,
    LowConfidence,
    TooFewLinesPlaced,
    NoVocalDetected,
    UnsupportedLanguage,
    LyricsMismatch,
    OutOfOrder,
    PlacedInSilence,
    BelowBaseline,
    InvalidRequest,
    UndecodableAudio,
    AudioTooLong,
    AudioTooLarge,
    TextTooLongForModel,
    HashMismatch,
    ModelOutputInvalid,
    DeadlineExceeded,
    EngineCrashed,
    OutOfMemory,
    DownloadFailed,
    ObjectStoreUnavailable,
    InternalError,
    EngineRestarted,
    WorkerLost,
    PublicNodeTimeout,
}

impl WorkerReason {
    pub const ALL: [Self; 33] = [
        Self::EmptyText,
        Self::EmptyReferenceText,
        Self::SilentAudio,
        Self::AudioTooShort,
        Self::EmptyVocab,
        Self::TooFewUsers,
        Self::AudioNotFound,
        Self::AudioForbidden,
        Self::ObjectNotFound,
        Self::LowConfidence,
        Self::TooFewLinesPlaced,
        Self::NoVocalDetected,
        Self::UnsupportedLanguage,
        Self::LyricsMismatch,
        Self::OutOfOrder,
        Self::PlacedInSilence,
        Self::BelowBaseline,
        Self::InvalidRequest,
        Self::UndecodableAudio,
        Self::AudioTooLong,
        Self::AudioTooLarge,
        Self::TextTooLongForModel,
        Self::HashMismatch,
        Self::ModelOutputInvalid,
        Self::DeadlineExceeded,
        Self::EngineCrashed,
        Self::OutOfMemory,
        Self::DownloadFailed,
        Self::ObjectStoreUnavailable,
        Self::InternalError,
        Self::EngineRestarted,
        Self::WorkerLost,
        Self::PublicNodeTimeout,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::EmptyText => "empty_text",
            Self::EmptyReferenceText => "empty_reference_text",
            Self::SilentAudio => "silent_audio",
            Self::AudioTooShort => "audio_too_short",
            Self::EmptyVocab => "empty_vocab",
            Self::TooFewUsers => "too_few_users",
            Self::AudioNotFound => "audio_not_found",
            Self::AudioForbidden => "audio_forbidden",
            Self::ObjectNotFound => "object_not_found",
            Self::LowConfidence => "low_confidence",
            Self::TooFewLinesPlaced => "too_few_lines_placed",
            Self::NoVocalDetected => "no_vocal_detected",
            Self::UnsupportedLanguage => "unsupported_language",
            Self::LyricsMismatch => "lyrics_mismatch",
            Self::OutOfOrder => "out_of_order",
            Self::PlacedInSilence => "placed_in_silence",
            Self::BelowBaseline => "below_baseline",
            Self::InvalidRequest => "invalid_request",
            Self::UndecodableAudio => "undecodable_audio",
            Self::AudioTooLong => "audio_too_long",
            Self::AudioTooLarge => "audio_too_large",
            Self::TextTooLongForModel => "text_too_long_for_model",
            Self::HashMismatch => "hash_mismatch",
            Self::ModelOutputInvalid => "model_output_invalid",
            Self::DeadlineExceeded => "deadline_exceeded",
            Self::EngineCrashed => "engine_crashed",
            Self::OutOfMemory => "out_of_memory",
            Self::DownloadFailed => "download_failed",
            Self::ObjectStoreUnavailable => "object_store_unavailable",
            Self::InternalError => "internal_error",
            Self::EngineRestarted => "engine_restarted",
            Self::WorkerLost => "worker_lost",
            Self::PublicNodeTimeout => "public_node_timeout",
        }
    }

    pub const fn status(self) -> WorkerStatus {
        match self {
            Self::EmptyText
            | Self::EmptyReferenceText
            | Self::SilentAudio
            | Self::AudioTooShort
            | Self::EmptyVocab
            | Self::TooFewUsers => WorkerStatus::Empty,
            Self::AudioNotFound | Self::AudioForbidden | Self::ObjectNotFound => {
                WorkerStatus::Missing
            }
            Self::LowConfidence
            | Self::TooFewLinesPlaced
            | Self::NoVocalDetected
            | Self::UnsupportedLanguage
            | Self::LyricsMismatch
            | Self::OutOfOrder
            | Self::PlacedInSilence
            | Self::BelowBaseline => WorkerStatus::Rejected,
            _ => WorkerStatus::Failed,
        }
    }

    pub const fn failure_class(self) -> Option<FailureClass> {
        match self {
            Self::InvalidRequest
            | Self::UndecodableAudio
            | Self::AudioTooLong
            | Self::AudioTooLarge
            | Self::TextTooLongForModel
            | Self::HashMismatch
            | Self::ModelOutputInvalid => Some(FailureClass::Deterministic),
            Self::DeadlineExceeded
            | Self::EngineCrashed
            | Self::OutOfMemory
            | Self::DownloadFailed
            | Self::ObjectStoreUnavailable
            | Self::InternalError => Some(FailureClass::Transient),
            Self::EngineRestarted | Self::WorkerLost | Self::PublicNodeTimeout => {
                Some(FailureClass::Reopenable)
            }
            _ => None,
        }
    }

    pub fn is_reopenable_on(self, lane: WorkerLane) -> bool {
        lane.reopenable().contains(&self)
    }

    pub fn is_published_on(self, lane: WorkerLane) -> bool {
        lane.reasons().contains(&self)
    }

    pub const SET_BY_BUS: [Self; 2] = [Self::WorkerLost, Self::PublicNodeTimeout];

    pub fn is_set_by_bus(self) -> bool {
        Self::SET_BY_BUS.contains(&self)
    }
}

impl WorkerLane {
    pub const fn reasons(self) -> &'static [WorkerReason] {
        match self {
            Self::Audio => &[
                WorkerReason::SilentAudio,
                WorkerReason::AudioTooShort,
                WorkerReason::AudioNotFound,
                WorkerReason::AudioForbidden,
                WorkerReason::InvalidRequest,
                WorkerReason::UndecodableAudio,
                WorkerReason::AudioTooLong,
                WorkerReason::AudioTooLarge,
                WorkerReason::ModelOutputInvalid,
                WorkerReason::DeadlineExceeded,
                WorkerReason::EngineCrashed,
                WorkerReason::OutOfMemory,
                WorkerReason::DownloadFailed,
                WorkerReason::InternalError,
                WorkerReason::EngineRestarted,
                WorkerReason::WorkerLost,
                WorkerReason::PublicNodeTimeout,
            ],
            Self::Lyrics => &[
                WorkerReason::EmptyText,
                WorkerReason::InvalidRequest,
                WorkerReason::TextTooLongForModel,
                WorkerReason::ModelOutputInvalid,
                WorkerReason::DeadlineExceeded,
                WorkerReason::EngineCrashed,
                WorkerReason::OutOfMemory,
                WorkerReason::InternalError,
                WorkerReason::EngineRestarted,
                WorkerReason::WorkerLost,
            ],
            Self::Transcribe => &[
                WorkerReason::EmptyReferenceText,
                WorkerReason::SilentAudio,
                WorkerReason::AudioNotFound,
                WorkerReason::AudioForbidden,
                WorkerReason::LowConfidence,
                WorkerReason::TooFewLinesPlaced,
                WorkerReason::NoVocalDetected,
                WorkerReason::UnsupportedLanguage,
                WorkerReason::LyricsMismatch,
                WorkerReason::OutOfOrder,
                WorkerReason::PlacedInSilence,
                WorkerReason::InvalidRequest,
                WorkerReason::UndecodableAudio,
                WorkerReason::AudioTooLong,
                WorkerReason::AudioTooLarge,
                WorkerReason::ModelOutputInvalid,
                WorkerReason::DeadlineExceeded,
                WorkerReason::EngineCrashed,
                WorkerReason::OutOfMemory,
                WorkerReason::DownloadFailed,
                WorkerReason::InternalError,
                WorkerReason::EngineRestarted,
                WorkerReason::WorkerLost,
                WorkerReason::PublicNodeTimeout,
            ],
            Self::Encode => &[
                WorkerReason::EmptyText,
                WorkerReason::InvalidRequest,
                WorkerReason::TextTooLongForModel,
                WorkerReason::HashMismatch,
                WorkerReason::ModelOutputInvalid,
                WorkerReason::DeadlineExceeded,
                WorkerReason::EngineCrashed,
                WorkerReason::OutOfMemory,
                WorkerReason::InternalError,
            ],
            Self::Collab => &[
                WorkerReason::EmptyVocab,
                WorkerReason::ObjectNotFound,
                WorkerReason::BelowBaseline,
                WorkerReason::InvalidRequest,
                WorkerReason::ModelOutputInvalid,
                WorkerReason::DeadlineExceeded,
                WorkerReason::EngineCrashed,
                WorkerReason::OutOfMemory,
                WorkerReason::ObjectStoreUnavailable,
                WorkerReason::InternalError,
                WorkerReason::EngineRestarted,
                WorkerReason::WorkerLost,
            ],
            Self::Taste => &[
                WorkerReason::TooFewUsers,
                WorkerReason::ObjectNotFound,
                WorkerReason::BelowBaseline,
                WorkerReason::InvalidRequest,
                WorkerReason::ModelOutputInvalid,
                WorkerReason::DeadlineExceeded,
                WorkerReason::EngineCrashed,
                WorkerReason::OutOfMemory,
                WorkerReason::ObjectStoreUnavailable,
                WorkerReason::InternalError,
                WorkerReason::EngineRestarted,
                WorkerReason::WorkerLost,
            ],
            Self::Ai => &[],
        }
    }

    pub fn worker_reasons(self) -> impl Iterator<Item = WorkerReason> {
        self.reasons()
            .iter()
            .copied()
            .filter(|reason| !reason.is_set_by_bus())
    }

    pub const fn reopenable(self) -> &'static [WorkerReason] {
        match self {
            Self::Audio | Self::Transcribe => &[
                WorkerReason::EngineRestarted,
                WorkerReason::WorkerLost,
                WorkerReason::PublicNodeTimeout,
            ],
            Self::Lyrics | Self::Collab | Self::Taste => {
                &[WorkerReason::EngineRestarted, WorkerReason::WorkerLost]
            }
            Self::Encode | Self::Ai => &[],
        }
    }
}

pub fn reasons_of(status: WorkerStatus) -> impl Iterator<Item = WorkerReason> {
    WorkerReason::ALL
        .into_iter()
        .filter(move |reason| reason.status() == status)
}

pub fn reasons_in(class: FailureClass) -> impl Iterator<Item = WorkerReason> {
    WorkerReason::ALL
        .into_iter()
        .filter(move |reason| reason.failure_class() == Some(class))
}

pub const fn outcome_rank(status: WorkerStatus, reason: Option<WorkerReason>) -> u8 {
    match (status, reason) {
        (WorkerStatus::Ok, _) => 4,
        (WorkerStatus::Failed, Some(reason)) => match reason.failure_class() {
            Some(FailureClass::Reopenable) => 1,
            Some(FailureClass::Transient) => 2,
            _ => 3,
        },
        _ => 3,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_reason_has_the_wire_name_serde_writes() {
        for reason in WorkerReason::ALL {
            assert_eq!(
                serde_json::to_value(reason).expect("reason serializes"),
                serde_json::Value::String(reason.as_str().to_owned())
            );
        }
        for status in WorkerStatus::ALL {
            assert_eq!(
                serde_json::to_value(status).expect("status serializes"),
                serde_json::Value::String(status.as_str().to_owned())
            );
        }
    }

    #[test]
    fn only_failed_reasons_have_a_failure_class() {
        for reason in WorkerReason::ALL {
            assert_eq!(
                reason.failure_class().is_some(),
                reason.status() == WorkerStatus::Failed,
                "{}",
                reason.as_str()
            );
        }
    }

    #[test]
    fn reopenable_sets_follow_the_design_per_lane() {
        let names = |lane: WorkerLane| {
            lane.reopenable()
                .iter()
                .map(|reason| reason.as_str())
                .collect::<Vec<_>>()
        };
        let three = ["engine_restarted", "worker_lost", "public_node_timeout"];
        let two = ["engine_restarted", "worker_lost"];

        assert_eq!(names(WorkerLane::Audio), three);
        assert_eq!(names(WorkerLane::Transcribe), three);
        assert_eq!(names(WorkerLane::Lyrics), two);
        assert_eq!(names(WorkerLane::Collab), two);
        assert_eq!(names(WorkerLane::Taste), two);
        assert!(names(WorkerLane::Encode).is_empty());
        assert!(names(WorkerLane::Ai).is_empty());
        for lane in WorkerLane::ALL {
            assert!(
                lane.reopenable()
                    .iter()
                    .all(|reason| reason.failure_class() == Some(FailureClass::Reopenable))
            );
        }
    }

    #[test]
    fn a_lane_publishes_no_reopenable_reason_outside_its_own_set() {
        assert!(!WorkerReason::PublicNodeTimeout.is_published_on(WorkerLane::Lyrics));
        assert!(!WorkerReason::EngineRestarted.is_published_on(WorkerLane::Encode));
        assert!(WorkerReason::PublicNodeTimeout.is_published_on(WorkerLane::Audio));
        assert!(WorkerReason::DeadlineExceeded.is_published_on(WorkerLane::Encode));
    }

    #[test]
    fn a_lane_publishes_only_the_reasons_of_its_own_section() {
        assert!(!WorkerReason::LyricsMismatch.is_published_on(WorkerLane::Encode));
        assert!(!WorkerReason::EmptyVocab.is_published_on(WorkerLane::Audio));
        assert!(!WorkerReason::BelowBaseline.is_published_on(WorkerLane::Audio));
        assert!(!WorkerReason::LowConfidence.is_published_on(WorkerLane::Audio));
        assert!(!WorkerReason::DownloadFailed.is_published_on(WorkerLane::Lyrics));
        assert!(!WorkerReason::BelowBaseline.is_published_on(WorkerLane::Transcribe));
        assert!(WorkerReason::LyricsMismatch.is_published_on(WorkerLane::Transcribe));
        assert!(WorkerReason::BelowBaseline.is_published_on(WorkerLane::Collab));
        assert!(WorkerReason::HashMismatch.is_published_on(WorkerLane::Encode));
    }

    #[test]
    fn a_lane_lists_its_reopenable_reasons_and_no_foreign_one() {
        for lane in WorkerLane::ALL {
            let reopenable: Vec<WorkerReason> = lane
                .reasons()
                .iter()
                .copied()
                .filter(|reason| reason.failure_class() == Some(FailureClass::Reopenable))
                .collect();
            assert_eq!(reopenable, lane.reopenable(), "{}", lane.as_str());
            let ordered = lane.reasons().windows(2).all(|pair| pair[0] < pair[1]);
            assert!(ordered, "{}", lane.as_str());
        }
    }

    #[test]
    fn outcome_rank_orders_ok_over_terminal_over_last_delivery_over_reopenable() {
        let ok = outcome_rank(WorkerStatus::Ok, None);
        let terminal = outcome_rank(WorkerStatus::Rejected, Some(WorkerReason::LowConfidence));
        let deterministic = outcome_rank(WorkerStatus::Failed, Some(WorkerReason::HashMismatch));
        let last_delivery =
            outcome_rank(WorkerStatus::Failed, Some(WorkerReason::DeadlineExceeded));
        let reopenable = outcome_rank(WorkerStatus::Failed, Some(WorkerReason::WorkerLost));

        assert_eq!(terminal, deterministic);
        assert!(ok > terminal && terminal > last_delivery && last_delivery > reopenable);
    }
}
