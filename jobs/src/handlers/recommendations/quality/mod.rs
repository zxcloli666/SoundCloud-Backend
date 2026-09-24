mod backfill;
mod features;
mod model;
mod store;
mod train;

use sqlx::PgPool;

use crate::qdrant::QdrantProvisioner;

pub(in crate::handlers) struct QualityHandler {
    pool: PgPool,
    qdrant: QdrantProvisioner,
}

impl QualityHandler {
    pub fn new(pool: PgPool, qdrant: QdrantProvisioner) -> Self {
        Self { pool, qdrant }
    }
}

#[cfg(test)]
mod tests;
