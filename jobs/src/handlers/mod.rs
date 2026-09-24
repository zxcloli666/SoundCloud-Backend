mod account_walk;
mod admin_maintenance;
mod ai_store;
mod attribution;
mod catalog;
mod catalog_collection;
mod catalog_credits;
#[cfg(test)]
#[path = "catalog_credits_tests.rs"]
mod catalog_credits_tests;
mod catalog_read;
mod catalog_refresh;
mod catalog_refresh_writer;
mod catalog_remote;
mod catalog_web_profiles;
mod collab;
mod crawl;
mod discover;
mod egress;
mod encode;
mod enrich;
mod external;
mod indexing;
mod lyrics;
mod maintenance;
mod oauth_apps;
mod oauth_cooldowns;
mod playlist_legacy;
#[cfg(test)]
#[path = "playlist_legacy_tests.rs"]
mod playlist_legacy_tests;
mod playlist_observe;
mod recommendations;
mod subscriptions;
mod sync_queue;
pub(crate) mod taste;
mod telemetry;
mod wanted;

use std::sync::Arc;

use backend_contracts::{
    AdminMaintenancePayload, CollabTrainPayload, CrawlArtistPayload, EmptyPayload, ImpressionBatch,
    IndexTrackPayload, JobKind, PlaylistObservePayload, StoredAudioDispatchPayload,
    SyncQueueFlushPayload, Versioned,
};
use serde::de::DeserializeOwned;

use crate::bus::Bus;
use crate::config::JobsConfig;
use crate::db::Databases;
use crate::qdrant::QdrantProvisioner;
use crate::queue::{JobError, JobResult, LeasedJob};

use self::account_walk::AccountWalkHandler;
use self::admin_maintenance::AdminMaintenanceHandler;
use self::attribution::AttributionHandler;
use self::catalog::CatalogWorkHandler;
use self::catalog_credits::CatalogCreditHandler;
use self::catalog_read::PublicCatalogReader;
use self::collab::{CollabHandler, CollabResult};
use self::crawl::CrawlHandler;
use self::discover::DiscoverHandler;
use self::encode::EncodeResultHandler;
use self::enrich::EnrichHandler;
use self::external::ExternalSources;
use self::indexing::IndexingHandler;
use self::lyrics::LyricsHandler;
use self::maintenance::MaintenanceHandler;
use self::oauth_apps::OAuthAppsRefreshHandler;
use self::playlist_legacy::PlaylistLegacyHandler;
use self::playlist_observe::PlaylistObserveHandler;
use self::recommendations::RecommendationHandler;
use self::recommendations::quality::QualityHandler;
use self::subscriptions::SubscriptionSnapshotHandler;
use self::sync_queue::SyncQueueHandler;
use self::telemetry::TelemetryHandler;
use self::wanted::WantedHandler;

pub(crate) const CORE_FAST_KINDS: &[JobKind] = &[
    JobKind::AuthCleanupLinkRequests,
    JobKind::AuthCleanupLoginRequests,
    JobKind::CleanupJobReceipts,
    JobKind::DispatchAudioIndex,
    JobKind::DispatchTranscription,
    JobKind::OAuthAppsRefresh,
    JobKind::SyncQueueFlush,
    JobKind::SyncQueueHeal,
];

pub(crate) const CORE_BULK_KINDS: &[JobKind] = &[
    JobKind::AdminCatalogRenormalize,
    JobKind::AdminMusicBrainzNames,
    JobKind::ArtistAttributionRevalidate,
    JobKind::CatalogCreditReview,
    JobKind::CatalogRefresh,
    JobKind::CatalogCollection,
    JobKind::CatalogWorkReconcile,
    JobKind::CollabBootstrap,
    JobKind::CollabTrain,
    JobKind::CrawlArtist,
    JobKind::DiscoverAccounts,
    JobKind::DiscoverAggregates,
    JobKind::DiscoverCatalogGenius,
    JobKind::DiscoverCatalogMusicBrainz,
    JobKind::DiscoverInterest,
    JobKind::EnrichTracks,
    JobKind::IndexTrack,
    JobKind::IndexingReap,
    JobKind::LyricsEmbed,
    JobKind::LyricsLookup,
    JobKind::LyricsLookupSweep,
    JobKind::LyricsReapEmbeddings,
    JobKind::LyricsReapTranscriptions,
    JobKind::PlaylistLegacyDrain,
    JobKind::PlaylistObserveShadow,
    JobKind::PlaylistReconcileSweep,
    JobKind::RecommendationColike,
    JobKind::RecommendationQualityBackfill,
    JobKind::RecommendationQualityTrain,
    JobKind::RecommendationWavePriority,
    JobKind::ResolveDurations,
    JobKind::ResolveWantedTracks,
    JobKind::SubscriptionsSnapshot,
];

pub(crate) const OPS_KINDS: &[JobKind] = &[JobKind::RecordHardNegative];

pub(crate) fn accepts_ingress(kind: JobKind) -> bool {
    matches!(
        kind,
        JobKind::AdminCatalogRenormalize
            | JobKind::AdminMusicBrainzNames
            | JobKind::CollabTrain
            | JobKind::CatalogRefresh
            | JobKind::CatalogCollection
            | JobKind::CrawlArtist
            | JobKind::DiscoverAggregates
            | JobKind::DispatchTranscription
            | JobKind::IndexTrack
            | JobKind::LyricsEmbed
            | JobKind::LyricsLookup
            | JobKind::PlaylistObserveShadow
            | JobKind::RecordHardNegative
            | JobKind::SyncQueueFlush
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkerLostLane {
    AudioIndex,
    Transcription,
    LyricsEmbedding,
}

pub struct JobHandlers {
    account_walk: AccountWalkHandler,
    admin_maintenance: AdminMaintenanceHandler,
    attribution: AttributionHandler,
    catalog: CatalogWorkHandler,
    catalog_credits: CatalogCreditHandler,
    catalog_refresh: catalog_refresh::CatalogRefreshHandler,
    catalog_collection: catalog_collection::CatalogCollectionHandler,
    collab: CollabHandler,
    crawl: CrawlHandler,
    discover: DiscoverHandler,
    enrich: EnrichHandler,
    encode: EncodeResultHandler,
    indexing: IndexingHandler,
    lyrics: LyricsHandler,
    maintenance: MaintenanceHandler,
    oauth_apps: OAuthAppsRefreshHandler,
    playlist_legacy: PlaylistLegacyHandler,
    playlist_observe: PlaylistObserveHandler,
    quality: QualityHandler,
    recommendations: RecommendationHandler,
    subscriptions: SubscriptionSnapshotHandler,
    sync_queue: SyncQueueHandler,
    telemetry: TelemetryHandler,
    wanted: Arc<WantedHandler>,
}

impl JobHandlers {
    pub fn new(
        databases: &Databases,
        config: &JobsConfig,
        bus: Bus,
        qdrant: QdrantProvisioner,
        relay: Option<std::sync::Arc<call_relay::Client>>,
    ) -> Result<Self, crate::ClientBuildError> {
        let sc = sc_transport::ScClient::new(&sc_transport::ScConfig {
            proxy_url: config.enrich.proxy_url.clone(),
            proxy_fallback: true,
            api_base: None,
            home_base: None,
        })?;
        let sc = match relay.clone() {
            Some(relay) => sc.with_relay(relay),
            None => sc,
        };
        let reader = Arc::new(PublicCatalogReader::new(sc, databases.main.fast.clone()));
        let sources = ExternalSources::build(relay, &config.enrich, &config.lyrics)?;
        let wanted = Arc::new(WantedHandler::new(
            databases.main.bulk.clone(),
            reader.clone(),
            bus.clone(),
            config.wanted.clone(),
        ));
        Ok(Self {
            account_walk: AccountWalkHandler::new(
                databases.main.bulk.clone(),
                reader.clone(),
                config.account_walk.clone(),
            ),
            admin_maintenance: AdminMaintenanceHandler::new(
                databases.main.bulk.clone(),
                sources.musicbrainz.clone(),
                config.admin_maintenance,
            ),
            attribution: AttributionHandler::new(databases.main.bulk.clone()),
            catalog: CatalogWorkHandler::new(databases.main.bulk.clone()),
            catalog_credits: CatalogCreditHandler::new(databases.main.bulk.clone()),
            catalog_collection: catalog_collection::CatalogCollectionHandler::new(
                databases.main.bulk.clone(),
                reader.clone(),
                config,
            )?,
            catalog_refresh: catalog_refresh::CatalogRefreshHandler::new(
                databases.main.bulk.clone(),
                reader.clone(),
                config,
            )?,
            collab: CollabHandler::new(
                databases.main.bulk.clone(),
                bus.clone(),
                qdrant.clone(),
                config.collab.clone(),
            ),
            crawl: CrawlHandler::new(
                databases.main.bulk.clone(),
                sources.musicbrainz.clone(),
                sources.genius.clone(),
                reader,
                wanted.clone(),
                config.crawl.clone(),
            ),
            discover: DiscoverHandler::new(
                databases.main.bulk.clone(),
                config.subscriptions_always_premium,
                config.schedules.discover_interest_enabled,
                config.schedules.discover_interest_shards,
                config.schedules.discover_artist_plays_shards,
            ),
            enrich: EnrichHandler::new(
                databases.main.bulk.clone(),
                enrich::build_resolver_deps(
                    databases.main.bulk.clone(),
                    bus.clone(),
                    &sources,
                    &config.enrich,
                ),
                &config.enrich,
            ),
            encode: EncodeResultHandler::new(qdrant.clone()),
            indexing: IndexingHandler::new(
                databases.main.fast.clone(),
                &config.indexing,
                &config.durations,
                &config.sync_queue.storage_url,
                bus.clone(),
                qdrant.clone(),
            )?
            .with_audio_dispatch(config.worker_dispatch.index_audio),
            lyrics: LyricsHandler::new(
                databases.main.fast.clone(),
                databases.main.bulk.clone(),
                sources.lyrics.clone(),
                config,
                bus.clone(),
                qdrant.clone(),
            ),
            maintenance: MaintenanceHandler::new(databases.main.fast.clone()),
            oauth_apps: OAuthAppsRefreshHandler::new(databases.main.fast.clone(), &config.oauth)?,
            playlist_legacy: PlaylistLegacyHandler::new(
                databases.main.bulk.clone(),
                config.playlist_reconcile,
            ),
            playlist_observe: PlaylistObserveHandler::new(config, databases.main.bulk.clone())?,
            quality: QualityHandler::new(databases.main.bulk.clone(), qdrant),
            recommendations: RecommendationHandler::new(
                databases.main.bulk.clone(),
                config.schedules.recommendation_wave_priority_shards,
            ),
            subscriptions: SubscriptionSnapshotHandler::new(
                databases.main.bulk.clone(),
                &config.subscriptions,
            ),
            sync_queue: SyncQueueHandler::new(config, databases.main.fast.clone())?,
            telemetry: TelemetryHandler::new(
                databases.ops.bulk.clone(),
                databases.ops.fast.clone(),
            ),
            wanted,
        })
    }

    pub async fn bootstrap(&self) -> JobResult {
        self.oauth_apps.bootstrap().await?;
        self.subscriptions.bootstrap().await
    }

    pub async fn record_impressions(&self, batch: ImpressionBatch) -> JobResult {
        self.telemetry.record_impressions(&batch).await
    }

    pub async fn finish_collab(&self, result: CollabResult) -> JobResult {
        self.collab.finish(result).await
    }

    pub async fn finish_lyrics_embedding(
        &self,
        result: backend_contracts::pipeline::LyricsEmbeddingResult,
        delivery: crate::bus::DeliveryContext,
    ) -> JobResult {
        self.lyrics.finish_embedding(result, delivery).await
    }

    pub async fn finish_transcription(
        &self,
        result: backend_contracts::pipeline::TranscriptionResult,
        delivery: crate::bus::DeliveryContext,
    ) -> JobResult {
        self.lyrics.finish_transcription(result, delivery).await
    }

    pub async fn finish_encode(
        &self,
        result: backend_contracts::pipeline::EncodeResult,
        delivery: crate::bus::DeliveryContext,
    ) -> JobResult {
        self.encode.finish(result, delivery).await
    }

    pub async fn finish_audio_index(
        &self,
        result: backend_contracts::pipeline::AudioIndexResult,
        delivery: crate::bus::DeliveryContext,
    ) -> JobResult {
        self.indexing.finish_audio_index(result, delivery).await
    }

    pub async fn apply_worker_lost(
        &self,
        lane: WorkerLostLane,
        stream_seq: u64,
        payload: &[u8],
    ) -> JobResult {
        match lane {
            WorkerLostLane::AudioIndex => {
                self.indexing.apply_worker_lost(stream_seq, payload).await
            }
            WorkerLostLane::Transcription => {
                self.lyrics.apply_worker_lost(stream_seq, payload).await
            }
            WorkerLostLane::LyricsEmbedding => {
                self.lyrics
                    .apply_embedding_worker_lost(stream_seq, payload)
                    .await
            }
        }
    }

    pub async fn reject_storage(
        &self,
        rejection: backend_contracts::pipeline::StorageTrackRejected,
        delivery: crate::bus::DeliveryContext,
    ) -> JobResult {
        self.indexing.reject_storage(rejection, delivery).await
    }

    pub async fn accept_storage_upload(
        &self,
        upload: backend_contracts::pipeline::StorageTrackUploaded,
        delivery: crate::bus::DeliveryContext,
    ) -> JobResult {
        self.indexing.accept_storage_upload(upload, delivery).await
    }

    pub async fn handle(&self, job: &LeasedJob) -> JobResult {
        match job.kind {
            JobKind::CatalogCollection => {
                self.catalog_collection
                    .refresh(
                        job,
                        payload::<backend_contracts::CatalogCollectionPayload>(job)?,
                    )
                    .await
            }
            JobKind::CatalogRefresh => {
                self.catalog_refresh
                    .refresh(
                        job,
                        payload::<backend_contracts::CatalogRefreshPayload>(job)?,
                    )
                    .await
            }
            JobKind::AuthCleanupLinkRequests => {
                empty_payload(job)?;
                self.maintenance.cleanup_link_requests().await
            }
            JobKind::AuthCleanupLoginRequests => {
                empty_payload(job)?;
                self.maintenance.cleanup_login_requests().await
            }
            JobKind::AdminCatalogRenormalize => {
                let payload = payload::<AdminMaintenancePayload>(job)?;
                self.admin_maintenance
                    .renormalize_catalog(job, payload)
                    .await
            }
            JobKind::AdminMusicBrainzNames => {
                let payload = payload::<AdminMaintenancePayload>(job)?;
                self.admin_maintenance
                    .reconcile_musicbrainz_names(job, payload)
                    .await
            }
            JobKind::ArtistAttributionRevalidate => {
                empty_payload(job)?;
                self.attribution.revalidate().await
            }
            JobKind::CatalogWorkReconcile => {
                empty_payload(job)?;
                self.catalog.reconcile().await
            }
            JobKind::CatalogCreditReview => {
                empty_payload(job)?;
                self.catalog_credits.review().await
            }
            JobKind::CleanupJobReceipts => {
                empty_payload(job)?;
                self.maintenance.cleanup_job_receipts().await
            }
            JobKind::CollabBootstrap => {
                empty_payload(job)?;
                self.collab.bootstrap(job.id).await
            }
            JobKind::CollabTrain => {
                let payload = payload::<CollabTrainPayload>(job)?;
                self.collab.train(job.id, payload).await
            }
            JobKind::OAuthAppsRefresh => {
                empty_payload(job)?;
                self.oauth_apps.refresh_due().await
            }
            JobKind::PlaylistObserveShadow => {
                let payload = payload::<PlaylistObservePayload>(job)?;
                self.playlist_observe
                    .observe(job.id, job.generation, payload)
                    .await
            }
            JobKind::PlaylistReconcileSweep => {
                empty_payload(job)?;
                self.playlist_observe.sweep_due().await
            }
            JobKind::PlaylistLegacyDrain => {
                empty_payload(job)?;
                self.playlist_legacy.drain().await
            }
            JobKind::DiscoverAccounts => {
                empty_payload(job)?;
                self.account_walk.run().await
            }
            JobKind::DiscoverAggregates => {
                empty_payload(job)?;
                self.discover.refresh_aggregates().await
            }
            JobKind::CrawlArtist => {
                let payload = payload::<CrawlArtistPayload>(job)?;
                self.crawl.crawl_artist(payload.artist_id).await
            }
            JobKind::DiscoverCatalogGenius => {
                empty_payload(job)?;
                self.crawl.crawl_genius_lane().await
            }
            JobKind::DiscoverCatalogMusicBrainz => {
                empty_payload(job)?;
                self.crawl.crawl_musicbrainz_lane().await
            }
            JobKind::ResolveWantedTracks => {
                empty_payload(job)?;
                self.wanted.resolve_due().await
            }
            JobKind::DiscoverInterest => {
                empty_payload(job)?;
                self.discover.recompute_interest().await
            }
            JobKind::EnrichTracks => {
                empty_payload(job)?;
                self.enrich.run().await
            }
            JobKind::DispatchAudioIndex => {
                let payload = payload::<StoredAudioDispatchPayload>(job)?;
                self.indexing.dispatch_audio(payload).await
            }
            JobKind::DispatchTranscription => {
                let payload = payload::<StoredAudioDispatchPayload>(job)?;
                self.lyrics.dispatch_transcription(payload).await
            }
            JobKind::IndexTrack => {
                let payload = payload::<IndexTrackPayload>(job)?;
                self.indexing.index_track(payload).await
            }
            JobKind::IndexingReap => {
                empty_payload(job)?;
                self.indexing.reap().await
            }
            JobKind::LyricsEmbed => {
                let payload = payload::<backend_contracts::LyricsEmbedPayload>(job)?;
                self.lyrics.embed(payload).await
            }
            JobKind::LyricsLookup => {
                let payload = payload::<backend_contracts::LyricsLookupPayload>(job)?;
                self.lyrics.lookup(job, payload).await
            }
            JobKind::LyricsLookupSweep => {
                empty_payload(job)?;
                self.lyrics.sweep_lookups(job).await
            }
            JobKind::LyricsReapEmbeddings => {
                empty_payload(job)?;
                self.lyrics.reap_embeddings().await
            }
            JobKind::LyricsReapTranscriptions => {
                empty_payload(job)?;
                self.lyrics.reap_transcriptions().await
            }
            JobKind::ResolveDurations => {
                empty_payload(job)?;
                self.indexing.resolve_durations().await
            }
            JobKind::RecordHardNegative => self.telemetry.record_hard_negative(job).await,
            JobKind::RecommendationColike => {
                empty_payload(job)?;
                self.recommendations.rebuild_colike().await
            }
            JobKind::RecommendationQualityBackfill => {
                empty_payload(job)?;
                self.quality.backfill().await
            }
            JobKind::RecommendationQualityTrain => {
                empty_payload(job)?;
                self.quality.train().await
            }
            JobKind::RecommendationWavePriority => {
                empty_payload(job)?;
                self.recommendations.bump_wave_priority().await
            }
            JobKind::SubscriptionsSnapshot => {
                empty_payload(job)?;
                self.subscriptions.export().await
            }
            JobKind::SyncQueueFlush => {
                let payload = payload::<SyncQueueFlushPayload>(job)?;
                self.sync_queue.flush(payload).await
            }
            JobKind::SyncQueueHeal => {
                empty_payload(job)?;
                self.maintenance.heal_sync_queue().await
            }
        }
    }
}

pub(super) fn payload<T>(job: &LeasedJob) -> JobResult<T>
where
    T: DeserializeOwned,
{
    serde_json::from_value::<Versioned<T>>(job.payload.clone())
        .map(Versioned::into_latest)
        .map_err(JobError::permanent)
}

fn empty_payload(job: &LeasedJob) -> JobResult<()> {
    payload::<EmptyPayload>(job).map(|_| ())
}

#[cfg(test)]
mod kind_tests {
    use super::*;

    #[test]
    fn every_job_kind_belongs_to_exactly_one_lane() {
        for kind in JobKind::ALL {
            let count = [CORE_FAST_KINDS, CORE_BULK_KINDS, OPS_KINDS]
                .into_iter()
                .filter(|kinds| kinds.contains(kind))
                .count();
            assert_eq!(count, 1, "{} belongs to {count} executable lanes", kind);
            assert_eq!(kind.lane().as_str(), lane_of(*kind));
        }
    }

    fn lane_of(kind: JobKind) -> &'static str {
        if CORE_FAST_KINDS.contains(&kind) {
            "core_fast"
        } else if CORE_BULK_KINDS.contains(&kind) {
            "core_bulk"
        } else {
            "ops"
        }
    }
}
#[cfg(test)]
mod catalog_metadata_tests;
