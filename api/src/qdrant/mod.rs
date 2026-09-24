use std::sync::Arc;
use std::time::Duration;

use qdrant_client::Qdrant;
use qdrant_client::config::QdrantConfig;
use qdrant_client::qdrant::{
    GetPointsBuilder, PointId, value::Kind as ValueKind, vector_output::Vector as VectorVariant,
    vectors_output::VectorsOptions,
};

use crate::config::QdrantCfg;
use crate::error::{AppError, AppResult};

mod bootstrap;

pub mod collections {
    pub use backend_contracts::vector_store::{
        QUERY_VEC_LYRICS, QUERY_VEC_MULAN, TRACKS_CLAP, TRACKS_COLLAB, TRACKS_LYRICS, TRACKS_MERT,
    };
}

pub struct QdrantService {
    client: Qdrant,
}

impl QdrantService {
    pub fn connect(cfg: &QdrantCfg) -> AppResult<Arc<Self>> {
        let mut qcfg = QdrantConfig::from_url(&cfg.grpc_url)
            .timeout(Duration::from_secs(5))
            .connect_timeout(Duration::from_secs(5))
            .skip_compatibility_check();
        if !cfg.api_key.is_empty() {
            qcfg = qcfg.api_key(cfg.api_key.expose().clone());
        }
        let client = Qdrant::new(qcfg)
            .map_err(|e| AppError::internal(format!("qdrant client init: {e}")))?;
        Ok(Arc::new(Self { client }))
    }

    pub fn raw(&self) -> &Qdrant {
        &self.client
    }

    pub async fn prepare_required_collections(&self) -> AppResult<()> {
        bootstrap::prepare_required_collections(&self.client).await
    }
    pub async fn get_query_vector(
        &self,
        collection: &str,
        hash: &str,
    ) -> Option<StoredQueryVector> {
        let started = std::time::Instant::now();
        let found = self.get_query_vector_inner(collection, hash).await;
        crate::metrics::record_dependency(
            "qdrant",
            "get_query_vector",
            if found.is_some() {
                crate::metrics::Outcome::Ok
            } else {
                crate::metrics::Outcome::Miss
            },
            started.elapsed(),
        );
        found
    }

    async fn get_query_vector_inner(
        &self,
        collection: &str,
        hash: &str,
    ) -> Option<StoredQueryVector> {
        let resp = self
            .client
            .get_points(
                GetPointsBuilder::new(collection, vec![query_point_id(hash)])
                    .with_vectors(true)
                    .with_payload(true),
            )
            .await
            .ok()?;
        let p = resp.result.into_iter().next()?;
        let encoder = match p
            .payload
            .get(QUERY_VECTOR_ENCODER_FIELD)
            .and_then(|value| value.kind.as_ref())
        {
            Some(ValueKind::StringValue(encoder)) => Some(encoder.clone()),
            _ => None,
        };
        let vector = match p.vectors.and_then(|v| v.vectors_options)? {
            VectorsOptions::Vector(v) => match v.into_vector() {
                VectorVariant::Dense(dense) => dense.data,
                _ => return None,
            },
            _ => return None,
        };
        Some(StoredQueryVector { vector, encoder })
    }
}

pub const QUERY_VECTOR_ENCODER_FIELD: &str = "encoder";

pub struct StoredQueryVector {
    pub vector: Vec<f32>,
    pub encoder: Option<String>,
}

fn query_point_id(hash: &str) -> PointId {
    PointId::from(backend_contracts::vector_store::query_point_uuid(hash))
}
