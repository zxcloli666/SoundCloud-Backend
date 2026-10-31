use std::sync::Arc;

use admin_catalog::execute;
use backend_contracts::admin_catalog::{
    ADMIN_CATALOG_QUEUE, ADMIN_CATALOG_SUBJECT, AdminCatalogReply, AdminCatalogRequest,
};
use sqlx::PgPool;
use tokio_util::sync::CancellationToken;

use crate::bus::Bus;

const CONCURRENCY: usize = 16;

pub struct AdminCatalogRpc {
    pool: PgPool,
}

impl AdminCatalogRpc {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn serve(
        self: Arc<Self>,
        bus: Bus,
        cancellation: CancellationToken,
    ) -> anyhow::Result<()> {
        bus.serve(
            ADMIN_CATALOG_SUBJECT,
            ADMIN_CATALOG_QUEUE,
            CONCURRENCY,
            cancellation,
            move |request: AdminCatalogRequest| {
                let rpc = self.clone();
                async move { rpc.reply(request).await }
            },
        )
        .await
    }

    async fn reply(&self, request: AdminCatalogRequest) -> anyhow::Result<AdminCatalogReply> {
        Ok(execute(&self.pool, &request).await?)
    }
}
