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

use crate::bus::{Bus, DeliveryContext};
use crate::config::{JobsConfig, WorkerDispatchConfig};
use crate::db::Databases;
use crate::qdrant::QdrantProvisioner;
use crate::queue::{JobResult, LeasedJob};
use backend_contracts::pipeline::{LyricsEmbeddingResult, TranscriptionResult};
use backend_contracts::worker_contract::{LYRICS_LANE, TRANSCRIBE_LANE};
use backend_contracts::{LyricsEmbedPayload, LyricsLookupPayload, StoredAudioDispatchPayload};
use catalog_sources::LyricsSources;

use self::dispatch::TranscriptionDispatcher;
use self::embedding_job::LyricsEmbeddingJob;
use self::embedding_result::EmbeddingResultHandler;
use self::lookup::LyricsLookupHandler;
use self::reaper::LyricsReaper;
use self::transcription::TranscriptionResultHandler;
use super::worker_backlog::WorkerBacklog;

pub struct LyricsHandler {
    embedding_job: LyricsEmbeddingJob,
    embedding_results: EmbeddingResultHandler,
    lookup: LyricsLookupHandler,
    sweep: LyricsLookupHandler,
    reaper: LyricsReaper,
    backlog: WorkerBacklog,
    dispatch: WorkerDispatchConfig,
    transcription_dispatcher: TranscriptionDispatcher,
    transcription_results: TranscriptionResultHandler,
}

impl LyricsHandler {
    pub fn new(
        databases: &Databases,
        sources: Arc<LyricsSources>,
        config: &JobsConfig,
        bus: Bus,
        qdrant: QdrantProvisioner,
    ) -> Self {
        let dispatch = config.worker_dispatch;
        let fast_pool = databases.main.fast.clone();
        Self {
            embedding_job: LyricsEmbeddingJob::new(fast_pool.clone(), bus.clone()),
            embedding_results: EmbeddingResultHandler::new(fast_pool.clone(), Arc::new(qdrant)),
            lookup: LyricsLookupHandler::new(
                databases.main.bulk.clone(),
                sources.clone(),
                config.lyrics.clone(),
            ),
            sweep: LyricsLookupHandler::new(
                databases.main.bulk.clone(),
                sources,
                config.lyrics.clone(),
            ),
            reaper: LyricsReaper::new(fast_pool.clone(), dispatch),
            backlog: WorkerBacklog::new(bus.clone()),
            dispatch,
            transcription_dispatcher: TranscriptionDispatcher::new(
                fast_pool.clone(),
                bus,
                config.sync_queue.storage_url.clone(),
            ),
            transcription_results: TranscriptionResultHandler::new(fast_pool),
        }
    }

    pub async fn dispatch_transcription(&self, payload: StoredAudioDispatchPayload) -> JobResult {
        let room = self
            .backlog
            .room(&TRANSCRIBE_LANE, self.dispatch.transcribe_backlog)
            .await;
        self.transcription_dispatcher
            .dispatch_transcription(payload, room)
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
        self.sweep.run_sweep(job).await
    }

    pub async fn reap_transcriptions(&self) -> JobResult {
        let room = self
            .backlog
            .room(&TRANSCRIBE_LANE, self.dispatch.transcribe_backlog)
            .await;
        self.reaper.reap_transcriptions(room).await
    }

    pub async fn reap_embeddings(&self) -> JobResult {
        let room = self
            .backlog
            .room(&LYRICS_LANE, self.dispatch.lyrics_backlog)
            .await;
        self.reaper.reap_embeddings(room).await
    }
}
