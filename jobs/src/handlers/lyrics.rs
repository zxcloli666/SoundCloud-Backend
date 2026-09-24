mod dispatch;
mod embedding_job;
mod embedding_queue;
mod embedding_result;
mod lookup;
mod reaper;
mod text;
mod transcription;
mod vectors;
pub(super) mod wake;

use std::sync::Arc;

use backend_contracts::pipeline::{LyricsEmbeddingResult, TranscriptionResult};
use backend_contracts::{LyricsEmbedPayload, LyricsLookupPayload, StoredAudioDispatchPayload};
use catalog_sources::LyricsSources;
use sqlx::PgPool;

use crate::bus::{Bus, DeliveryContext};
use crate::config::JobsConfig;
use crate::qdrant::QdrantProvisioner;
use crate::queue::{JobResult, LeasedJob};

use self::dispatch::TranscriptionDispatcher;
use self::embedding_job::LyricsEmbeddingJob;
use self::embedding_result::EmbeddingResultHandler;
use self::lookup::LyricsLookupHandler;
use self::reaper::LyricsReaper;
use self::transcription::TranscriptionResultHandler;

pub struct LyricsHandler {
    embedding_job: LyricsEmbeddingJob,
    embedding_results: EmbeddingResultHandler,
    lookup: LyricsLookupHandler,
    reaper: LyricsReaper,
    transcription_dispatcher: TranscriptionDispatcher,
    transcription_results: TranscriptionResultHandler,
}

impl LyricsHandler {
    pub fn new(
        fast_pool: PgPool,
        bulk_pool: PgPool,
        sources: Arc<LyricsSources>,
        config: &JobsConfig,
        bus: Bus,
        qdrant: QdrantProvisioner,
    ) -> Self {
        let dispatch = config.worker_dispatch;
        Self {
            embedding_job: LyricsEmbeddingJob::new(
                fast_pool.clone(),
                bus.clone(),
                dispatch.embed_lyrics,
            ),
            embedding_results: EmbeddingResultHandler::new(fast_pool.clone(), Arc::new(qdrant)),
            lookup: LyricsLookupHandler::new(bulk_pool.clone(), sources, config.lyrics.clone()),
            reaper: LyricsReaper::new(bulk_pool, dispatch),
            transcription_dispatcher: TranscriptionDispatcher::new(
                fast_pool.clone(),
                bus,
                config.sync_queue.storage_url.clone(),
                dispatch.transcribe,
            ),
            transcription_results: TranscriptionResultHandler::new(fast_pool),
        }
    }

    pub async fn dispatch_transcription(&self, payload: StoredAudioDispatchPayload) -> JobResult {
        self.transcription_dispatcher
            .dispatch_transcription(payload)
            .await
    }

    pub async fn apply_worker_lost(&self, stream_seq: u64, payload: &[u8]) -> JobResult {
        self.transcription_results
            .apply_worker_lost(stream_seq, payload)
            .await
    }

    pub async fn apply_embedding_worker_lost(&self, stream_seq: u64, payload: &[u8]) -> JobResult {
        self.embedding_results
            .apply_worker_lost(stream_seq, payload)
            .await
    }

    pub async fn finish_transcription(
        &self,
        result: TranscriptionResult,
        delivery: DeliveryContext,
    ) -> JobResult {
        self.transcription_results.finish(result, delivery).await
    }

    pub async fn embed(&self, payload: LyricsEmbedPayload) -> JobResult {
        self.embedding_job.run(payload).await
    }

    pub async fn finish_embedding(
        &self,
        result: LyricsEmbeddingResult,
        delivery: DeliveryContext,
    ) -> JobResult {
        self.embedding_results.finish(result, delivery).await
    }

    pub async fn lookup(&self, job: &LeasedJob, payload: LyricsLookupPayload) -> JobResult {
        self.lookup.run_targeted(job, payload).await
    }

    pub async fn sweep_lookups(&self, job: &LeasedJob) -> JobResult {
        self.lookup.run_sweep(job).await
    }

    pub async fn reap_transcriptions(&self) -> JobResult {
        self.reaper.reap_transcriptions().await
    }

    pub async fn reap_embeddings(&self) -> JobResult {
        self.reaper.reap_embeddings().await
    }
}
