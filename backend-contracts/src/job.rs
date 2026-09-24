use std::fmt::{Display, Formatter};
use std::str::FromStr;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum JobLane {
    CoreFast,
    CoreBulk,
    Ops,
}

impl JobLane {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CoreFast => "core_fast",
            Self::CoreBulk => "core_bulk",
            Self::Ops => "ops",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct EmptyPayload {}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdminMaintenancePayload {
    pub run_id: uuid::Uuid,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct SyncQueueFlushPayload {
    #[serde(default)]
    pub force: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct IndexTrackPayload {
    pub sc_track_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct StoredAudioDispatchPayload {
    pub sc_track_id: String,
    pub uploaded_generation: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LyricsEmbedPayload {
    pub sc_track_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LyricsLookupPayload {
    pub sc_track_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PlaylistObservePayload {
    pub playlist_urn: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CrawlArtistPayload {
    pub artist_id: uuid::Uuid,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct CollabTrainPayload {
    #[serde(default)]
    pub min_count: Option<u32>,
}

pub const COLLAB_MAX_MIN_COUNT: u32 = 1_000_000;

pub const SYNC_QUEUE_MAX_RETRIES: i32 = 5;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobKind {
    AdminCatalogRenormalize,
    AdminMusicBrainzNames,
    ArtistAttributionRevalidate,
    AuthCleanupLinkRequests,
    AuthCleanupLoginRequests,
    CollabBootstrap,
    CollabTrain,
    CatalogCreditReview,
    CatalogRefresh,
    CatalogCollection,
    CatalogWorkReconcile,
    CleanupJobReceipts,
    CrawlArtist,
    DiscoverAccounts,
    DiscoverAggregates,
    DiscoverCatalogGenius,
    DiscoverCatalogMusicBrainz,
    DiscoverInterest,
    DispatchAudioIndex,
    DispatchTranscription,
    EnrichTracks,
    IndexTrack,
    IndexingReap,
    LyricsEmbed,
    LyricsLookup,
    LyricsLookupSweep,
    LyricsReapEmbeddings,
    LyricsReapTranscriptions,
    OAuthAppsRefresh,
    PlaylistLegacyDrain,
    PlaylistObserveShadow,
    PlaylistReconcileSweep,
    RecommendationColike,
    RecommendationQualityBackfill,
    RecommendationQualityTrain,
    RecommendationWavePriority,
    RecordHardNegative,
    ResolveDurations,
    ResolveWantedTracks,
    SubscriptionsSnapshot,
    SyncQueueFlush,
    SyncQueueHeal,
}

impl JobKind {
    pub const ALL: &'static [Self] = &[
        Self::AdminCatalogRenormalize,
        Self::AdminMusicBrainzNames,
        Self::ArtistAttributionRevalidate,
        Self::AuthCleanupLinkRequests,
        Self::AuthCleanupLoginRequests,
        Self::CollabBootstrap,
        Self::CollabTrain,
        Self::CatalogCreditReview,
        Self::CatalogRefresh,
        Self::CatalogCollection,
        Self::CatalogWorkReconcile,
        Self::CleanupJobReceipts,
        Self::CrawlArtist,
        Self::DiscoverAccounts,
        Self::DiscoverAggregates,
        Self::DiscoverCatalogGenius,
        Self::DiscoverCatalogMusicBrainz,
        Self::DiscoverInterest,
        Self::DispatchAudioIndex,
        Self::DispatchTranscription,
        Self::EnrichTracks,
        Self::IndexTrack,
        Self::IndexingReap,
        Self::LyricsEmbed,
        Self::LyricsLookup,
        Self::LyricsLookupSweep,
        Self::LyricsReapEmbeddings,
        Self::LyricsReapTranscriptions,
        Self::OAuthAppsRefresh,
        Self::PlaylistLegacyDrain,
        Self::PlaylistObserveShadow,
        Self::PlaylistReconcileSweep,
        Self::RecommendationColike,
        Self::RecommendationQualityBackfill,
        Self::RecommendationQualityTrain,
        Self::RecommendationWavePriority,
        Self::RecordHardNegative,
        Self::ResolveDurations,
        Self::ResolveWantedTracks,
        Self::SubscriptionsSnapshot,
        Self::SyncQueueFlush,
        Self::SyncQueueHeal,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AdminCatalogRenormalize => "admin.renormalize_catalog",
            Self::AdminMusicBrainzNames => "admin.reconcile_musicbrainz_names",
            Self::ArtistAttributionRevalidate => "enrich.revalidate_attribution",
            Self::AuthCleanupLinkRequests => "auth.cleanup_link_requests",
            Self::AuthCleanupLoginRequests => "auth.cleanup_login_requests",
            Self::CollabBootstrap => "collab.bootstrap",
            Self::CollabTrain => "collab.train",
            Self::CatalogCreditReview => "catalog.review_credits",
            Self::CatalogRefresh => "catalog.refresh",
            Self::CatalogCollection => "catalog.collection",
            Self::CatalogWorkReconcile => "catalog.reconcile_works",
            Self::CleanupJobReceipts => "jobs.cleanup_receipts",
            Self::CrawlArtist => "crawl.artist",
            Self::DiscoverAccounts => "discover.accounts",
            Self::DiscoverAggregates => "discover.aggregates",
            Self::DiscoverCatalogGenius => "discover.catalog_genius",
            Self::DiscoverCatalogMusicBrainz => "discover.catalog_musicbrainz",
            Self::DiscoverInterest => "discover.interest",
            Self::DispatchAudioIndex => "indexing.dispatch_audio",
            Self::DispatchTranscription => "lyrics.dispatch_transcription",
            Self::EnrichTracks => "enrich.tracks",
            Self::IndexTrack => "indexing.track",
            Self::IndexingReap => "indexing.reap",
            Self::LyricsEmbed => "lyrics.embed",
            Self::LyricsLookup => "lyrics.lookup",
            Self::LyricsLookupSweep => "lyrics.lookup_sweep",
            Self::LyricsReapEmbeddings => "lyrics.reap_embeddings",
            Self::LyricsReapTranscriptions => "lyrics.reap_transcriptions",
            Self::OAuthAppsRefresh => "oauth_apps.refresh",
            Self::PlaylistLegacyDrain => "playlists.legacy_drain",
            Self::PlaylistObserveShadow => "playlists.observe_shadow",
            Self::PlaylistReconcileSweep => "playlists.reconcile_sweep",
            Self::RecommendationColike => "recommendations.colike",
            Self::RecommendationQualityBackfill => "recommendations.quality_backfill",
            Self::RecommendationQualityTrain => "recommendations.quality_train",
            Self::RecommendationWavePriority => "recommendations.wave_priority",
            Self::RecordHardNegative => "telemetry.hard_negative",
            Self::ResolveDurations => "indexing.resolve_durations",
            Self::ResolveWantedTracks => "enrich.resolve_wanted",
            Self::SubscriptionsSnapshot => "subscriptions.snapshot",
            Self::SyncQueueFlush => "sync_queue.flush",
            Self::SyncQueueHeal => "sync_queue.heal",
        }
    }

    pub const fn lane(self) -> JobLane {
        match self {
            Self::AuthCleanupLinkRequests
            | Self::AuthCleanupLoginRequests
            | Self::CleanupJobReceipts
            | Self::DispatchAudioIndex
            | Self::DispatchTranscription
            | Self::OAuthAppsRefresh
            | Self::SyncQueueFlush
            | Self::SyncQueueHeal => JobLane::CoreFast,
            Self::RecordHardNegative => JobLane::Ops,
            Self::AdminCatalogRenormalize
            | Self::AdminMusicBrainzNames
            | Self::ArtistAttributionRevalidate
            | Self::CatalogCreditReview
            | Self::CatalogRefresh
            | Self::CatalogCollection
            | Self::CatalogWorkReconcile
            | Self::CollabBootstrap
            | Self::CollabTrain
            | Self::CrawlArtist
            | Self::DiscoverAccounts
            | Self::DiscoverAggregates
            | Self::DiscoverCatalogGenius
            | Self::DiscoverCatalogMusicBrainz
            | Self::DiscoverInterest
            | Self::EnrichTracks
            | Self::IndexTrack
            | Self::IndexingReap
            | Self::LyricsEmbed
            | Self::LyricsLookup
            | Self::LyricsLookupSweep
            | Self::LyricsReapEmbeddings
            | Self::LyricsReapTranscriptions
            | Self::PlaylistLegacyDrain
            | Self::PlaylistObserveShadow
            | Self::PlaylistReconcileSweep
            | Self::RecommendationColike
            | Self::RecommendationQualityBackfill
            | Self::RecommendationQualityTrain
            | Self::RecommendationWavePriority
            | Self::ResolveDurations
            | Self::ResolveWantedTracks
            | Self::SubscriptionsSnapshot => JobLane::CoreBulk,
        }
    }
}

impl Display for JobKind {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for JobKind {
    type Err = UnknownJobKind;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let kind = match value {
            "admin.renormalize_catalog" => Self::AdminCatalogRenormalize,
            "admin.reconcile_musicbrainz_names" => Self::AdminMusicBrainzNames,
            "enrich.revalidate_attribution" => Self::ArtistAttributionRevalidate,
            "auth.cleanup_link_requests" => Self::AuthCleanupLinkRequests,
            "auth.cleanup_login_requests" => Self::AuthCleanupLoginRequests,
            "collab.bootstrap" => Self::CollabBootstrap,
            "collab.train" => Self::CollabTrain,
            "catalog.review_credits" => Self::CatalogCreditReview,
            "catalog.refresh" => Self::CatalogRefresh,
            "catalog.collection" => Self::CatalogCollection,
            "catalog.reconcile_works" => Self::CatalogWorkReconcile,
            "jobs.cleanup_receipts" => Self::CleanupJobReceipts,
            "crawl.artist" => Self::CrawlArtist,
            "discover.accounts" => Self::DiscoverAccounts,
            "discover.aggregates" => Self::DiscoverAggregates,
            "discover.catalog_genius" => Self::DiscoverCatalogGenius,
            "discover.catalog_musicbrainz" => Self::DiscoverCatalogMusicBrainz,
            "discover.interest" => Self::DiscoverInterest,
            "indexing.dispatch_audio" => Self::DispatchAudioIndex,
            "lyrics.dispatch_transcription" => Self::DispatchTranscription,
            "enrich.tracks" => Self::EnrichTracks,
            "indexing.track" => Self::IndexTrack,
            "indexing.reap" => Self::IndexingReap,
            "lyrics.embed" => Self::LyricsEmbed,
            "lyrics.lookup" => Self::LyricsLookup,
            "lyrics.lookup_sweep" => Self::LyricsLookupSweep,
            "lyrics.reap_embeddings" => Self::LyricsReapEmbeddings,
            "lyrics.reap_transcriptions" => Self::LyricsReapTranscriptions,
            "oauth_apps.refresh" => Self::OAuthAppsRefresh,
            "playlists.legacy_drain" => Self::PlaylistLegacyDrain,
            "playlists.observe_shadow" => Self::PlaylistObserveShadow,
            "playlists.reconcile_sweep" => Self::PlaylistReconcileSweep,
            "recommendations.colike" => Self::RecommendationColike,
            "recommendations.quality_backfill" => Self::RecommendationQualityBackfill,
            "recommendations.quality_train" => Self::RecommendationQualityTrain,
            "recommendations.wave_priority" => Self::RecommendationWavePriority,
            "telemetry.hard_negative" => Self::RecordHardNegative,
            "indexing.resolve_durations" => Self::ResolveDurations,
            "enrich.resolve_wanted" => Self::ResolveWantedTracks,
            "subscriptions.snapshot" => Self::SubscriptionsSnapshot,
            "sync_queue.flush" => Self::SyncQueueFlush,
            "sync_queue.heal" => Self::SyncQueueHeal,
            _ => return Err(UnknownJobKind(value.to_owned())),
        };
        Ok(kind)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnknownJobKind(String);

impl Display for UnknownJobKind {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "unknown job kind {:?}", self.0)
    }
}

impl std::error::Error for UnknownJobKind {}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "version", content = "payload")]
pub enum Versioned<T> {
    #[serde(rename = "1")]
    V1(T),
}

impl<T> Versioned<T> {
    pub fn into_latest(self) -> T {
        match self {
            Self::V1(value) => value,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_round_trips_through_its_wire_name() {
        let kinds = JobKind::ALL;
        let legacy = [
            JobKind::AdminCatalogRenormalize,
            JobKind::AdminMusicBrainzNames,
            JobKind::ArtistAttributionRevalidate,
            JobKind::AuthCleanupLinkRequests,
            JobKind::AuthCleanupLoginRequests,
            JobKind::CollabBootstrap,
            JobKind::CollabTrain,
            JobKind::CatalogWorkReconcile,
            JobKind::CleanupJobReceipts,
            JobKind::CrawlArtist,
            JobKind::DiscoverAccounts,
            JobKind::DiscoverAggregates,
            JobKind::DiscoverCatalogGenius,
            JobKind::DiscoverCatalogMusicBrainz,
            JobKind::DiscoverInterest,
            JobKind::DispatchAudioIndex,
            JobKind::DispatchTranscription,
            JobKind::EnrichTracks,
            JobKind::IndexTrack,
            JobKind::IndexingReap,
            JobKind::LyricsEmbed,
            JobKind::LyricsLookup,
            JobKind::LyricsLookupSweep,
            JobKind::LyricsReapEmbeddings,
            JobKind::LyricsReapTranscriptions,
            JobKind::OAuthAppsRefresh,
            JobKind::PlaylistObserveShadow,
            JobKind::RecommendationColike,
            JobKind::RecommendationQualityBackfill,
            JobKind::RecommendationQualityTrain,
            JobKind::RecommendationWavePriority,
            JobKind::RecordHardNegative,
            JobKind::ResolveDurations,
            JobKind::ResolveWantedTracks,
            JobKind::SubscriptionsSnapshot,
            JobKind::SyncQueueFlush,
            JobKind::SyncQueueHeal,
        ];

        for kind in kinds {
            assert_eq!(kind.as_str().parse(), Ok(*kind));
        }
        for kind in legacy {
            assert!(kinds.contains(&kind));
        }
        let names: std::collections::HashSet<&str> =
            kinds.iter().map(|kind| kind.as_str()).collect();
        assert_eq!(names.len(), kinds.len());
        assert_eq!(kinds.len(), 42);
    }

    #[test]
    fn latency_sensitive_and_ops_kinds_have_dedicated_lanes() {
        assert_eq!(JobKind::AuthCleanupLoginRequests.lane(), JobLane::CoreFast);
        assert_eq!(JobKind::DispatchAudioIndex.lane(), JobLane::CoreFast);
        assert_eq!(JobKind::DispatchTranscription.lane(), JobLane::CoreFast);
        assert_eq!(JobKind::LyricsEmbed.lane(), JobLane::CoreBulk);
        assert_eq!(JobKind::DiscoverAggregates.lane(), JobLane::CoreBulk);
        assert_eq!(JobKind::PlaylistObserveShadow.lane(), JobLane::CoreBulk);
        assert_eq!(JobKind::RecordHardNegative.lane(), JobLane::Ops);
    }

    #[test]
    fn versioned_payload_has_stable_wire_shape() {
        let payload = Versioned::V1(EmptyPayload {});

        assert_eq!(
            serde_json::to_value(payload).expect("empty payload should serialize"),
            serde_json::json!({ "version": "1", "payload": {} })
        );
    }

    #[test]
    fn sync_queue_schedule_payload_defaults_to_a_due_flush() {
        let payload: Versioned<SyncQueueFlushPayload> =
            serde_json::from_value(serde_json::json!({ "version": "1", "payload": {} }))
                .expect("sync queue payload");

        assert_eq!(
            payload.into_latest(),
            SyncQueueFlushPayload { force: false }
        );
    }

    #[test]
    fn index_track_payload_has_a_stable_wire_shape() {
        let payload = Versioned::V1(IndexTrackPayload {
            sc_track_id: "42".to_owned(),
        });

        assert_eq!(
            serde_json::to_value(payload).expect("index track payload should serialize"),
            serde_json::json!({
                "version": "1",
                "payload": { "sc_track_id": "42" }
            })
        );
    }

    #[test]
    fn stored_audio_dispatch_payload_has_a_stable_wire_shape() {
        let payload = Versioned::V1(StoredAudioDispatchPayload {
            sc_track_id: "42".to_owned(),
            uploaded_generation: 3,
        });

        assert_eq!(
            serde_json::to_value(payload).expect("stored audio dispatch payload should serialize"),
            serde_json::json!({
                "version": "1",
                "payload": {
                    "sc_track_id": "42",
                    "uploaded_generation": 3
                }
            })
        );
    }

    #[test]
    fn lyrics_embed_payload_has_a_stable_wire_shape() {
        let payload = Versioned::V1(LyricsEmbedPayload {
            sc_track_id: "42".to_owned(),
        });

        assert_eq!(
            serde_json::to_value(payload).expect("lyrics embed payload should serialize"),
            serde_json::json!({
                "version": "1",
                "payload": { "sc_track_id": "42" }
            })
        );
    }

    #[test]
    fn lyrics_lookup_payload_has_a_stable_wire_shape() {
        let payload = Versioned::V1(LyricsLookupPayload {
            sc_track_id: "42".to_owned(),
        });

        assert_eq!(
            serde_json::to_value(payload).expect("lyrics lookup payload should serialize"),
            serde_json::json!({
                "version": "1",
                "payload": { "sc_track_id": "42" }
            })
        );
    }

    #[test]
    fn lyrics_lookup_payload_rejects_unknown_fields() {
        let payload = serde_json::from_value::<Versioned<LyricsLookupPayload>>(serde_json::json!({
            "version": "1",
            "payload": { "sc_track_id": "42", "track_id": "ignored" }
        }));

        assert!(payload.is_err());
    }

    #[test]
    fn playlist_observe_payload_has_a_stable_wire_shape() {
        let payload = Versioned::V1(PlaylistObservePayload {
            playlist_urn: "soundcloud:playlists:42".to_owned(),
        });

        assert_eq!(
            serde_json::to_value(payload).expect("playlist observe payload should serialize"),
            serde_json::json!({
                "version": "1",
                "payload": { "playlist_urn": "soundcloud:playlists:42" }
            })
        );
    }

    #[test]
    fn collab_schedule_payload_uses_configured_defaults() {
        let payload: Versioned<CollabTrainPayload> =
            serde_json::from_value(serde_json::json!({ "version": "1", "payload": {} }))
                .expect("collab payload should deserialize");

        assert_eq!(payload.into_latest(), CollabTrainPayload::default());
    }

    #[test]
    fn a_stored_collab_payload_with_a_dimension_still_deserializes() {
        let payload: Versioned<CollabTrainPayload> = serde_json::from_value(serde_json::json!({
            "version": "1",
            "payload": { "dim": 64, "min_count": 7 }
        }))
        .expect("v1 collab payload with dim should deserialize");

        assert_eq!(
            payload.into_latest(),
            CollabTrainPayload { min_count: Some(7) }
        );
    }
}
