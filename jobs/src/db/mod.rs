#[cfg(test)]
mod connection_discipline_tests;
pub(crate) mod migrations;
mod pool;

use sqlx::PgPool;

use crate::config::JobsConfig;

pub use pool::DatabaseError;

#[derive(Clone)]
pub struct Databases {
    pub main: DatabasePools,
    pub queue: PgPool,
    pub maintenance: PgPool,
    pub ops: DatabasePools,
}

#[derive(Clone)]
pub struct DatabasePools {
    pub fast: PgPool,
    pub bulk: PgPool,
}

impl Databases {
    pub async fn connect(config: &JobsConfig) -> Result<Self, DatabaseError> {
        let main_name = format!("scd-jobs:{}:main", config.instance_id);
        let queue_name = format!("scd-jobs:{}:main:queue", config.instance_id);
        let maintenance_name = format!("scd-jobs:{}:main:maintenance", config.instance_id);
        let ops_name = format!("scd-jobs:{}:ops", config.instance_id);
        let main = DatabasePools::connect(&config.main_database, &main_name);
        let queue = pool::connect(&config.main_database, &config.queue_pool, &queue_name);
        let maintenance = pool::connect(
            &config.main_database,
            &config.maintenance_pool,
            &maintenance_name,
        );
        let ops = DatabasePools::connect(&config.ops_database, &ops_name);
        let (main, queue, maintenance, ops) = tokio::try_join!(main, queue, maintenance, ops)?;
        Ok(Self {
            main,
            queue,
            maintenance,
            ops,
        })
    }

    pub async fn close(&self) {
        tokio::join!(
            self.main.close(),
            self.queue.close(),
            self.maintenance.close(),
            self.ops.close()
        );
    }
}

pub(crate) async fn connect_migration(
    config: &crate::config::DatabaseConfig,
    application_name: &str,
) -> Result<PgPool, DatabaseError> {
    pool::connect(config, &config.bulk_pool, application_name).await
}

impl DatabasePools {
    async fn connect(
        config: &crate::config::DatabaseConfig,
        application_name: &str,
    ) -> Result<Self, DatabaseError> {
        let fast_name = format!("{application_name}:fast");
        let bulk_name = format!("{application_name}:bulk");
        let fast = pool::connect(config, &config.fast_pool, &fast_name);
        let bulk = pool::connect(config, &config.bulk_pool, &bulk_name);
        let (fast, bulk) = tokio::try_join!(fast, bulk)?;
        Ok(Self { fast, bulk })
    }

    async fn close(&self) {
        tokio::join!(self.fast.close(), self.bulk.close());
    }
}
