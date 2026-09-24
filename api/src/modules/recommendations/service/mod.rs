mod enrichment;
mod qdrant_io;
mod seed;
mod types;
pub(crate) mod util;
mod verify;

#[cfg(test)]
pub(crate) use qdrant_io::{LYRICS_VECTOR_REQUEST_FIELD, lyrics_vec_cache_key};
pub use types::RecommendResult;
pub(crate) use types::ScoredCandidate;

use std::sync::Arc;

use deadpool_redis::Pool as RedisPool;
use sqlx::PgPool;

use crate::bus::nats::NatsService;
use crate::cache::KeyedCoalesce;
use crate::config::SoundwaveCfg;
use crate::modules::collab::CollabVectorService;
use crate::modules::lyrics::WorkerClient;
use crate::modules::recommendations::s3_verifier::S3VerifierService;
use crate::qdrant::QdrantService;

pub struct RecommendationsService {
    pub(crate) qdrant: Arc<QdrantService>,
    pub(crate) pg: PgPool,
    pub(crate) nats: Arc<NatsService>,
    pub(crate) redis: RedisPool,
    pub(crate) worker: Arc<WorkerClient>,
    pub(crate) s3: Arc<S3VerifierService>,
    pub(crate) collab: Arc<CollabVectorService>,
    pub(crate) cfg: SoundwaveCfg,
    pub(crate) cluster_flights: KeyedCoalesce<String>,
}

impl RecommendationsService {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        qdrant: Arc<QdrantService>,
        pg: PgPool,
        nats: Arc<NatsService>,
        redis: RedisPool,
        worker: Arc<WorkerClient>,
        s3: Arc<S3VerifierService>,
        collab: Arc<CollabVectorService>,
        cfg: SoundwaveCfg,
    ) -> Arc<Self> {
        Arc::new(Self {
            qdrant,
            pg,
            nats,
            redis,
            worker,
            s3,
            collab,
            cfg,
            cluster_flights: KeyedCoalesce::new(),
        })
    }
}
