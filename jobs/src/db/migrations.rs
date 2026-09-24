use std::borrow::Cow;

use sqlx::PgPool;

use crate::config::OAuthAppBootstrap;

const CORE_LOCK: i64 = 0x5343_445F_4D49;
const OPS_LOCK: i64 = 0x5343_445F_4D4F;
const CONNECTION_MIGRATION: i64 = 59;

static CORE: sqlx::migrate::Migrator = {
    let mut migrator = sqlx::migrate!("../api/migrations");
    migrator.ignore_missing = true;
    migrator
};

static OPS: sqlx::migrate::Migrator = {
    let mut migrator = sqlx::migrate!("../api/migrations-ops");
    migrator.ignore_missing = true;
    migrator
};

pub(crate) async fn run_core(
    pool: &PgPool,
    bootstrap_app: Option<&OAuthAppBootstrap>,
) -> Result<(), MigrationError> {
    let mut connection = pool.acquire().await?;
    prepare(&mut connection, CORE_LOCK).await?;

    let migration = async {
        core_prefix()
            .run(&mut *connection)
            .await
            .map_err(MigrationError::Migration)?;
        preflight_oauth_app_identities(&mut connection).await?;
        migrate_environment_connections(&mut connection, bootstrap_app).await?;
        CORE.run(&mut *connection)
            .await
            .map_err(MigrationError::Migration)
    }
    .await;
    finish(&mut connection, CORE_LOCK, migration).await
}

async fn preflight_oauth_app_identities(
    connection: &mut sqlx::PgConnection,
) -> Result<(), MigrationError> {
    let invalid = sqlx::query_file!("queries/oauth_apps/identity_preflight.sql")
        .fetch_one(&mut *connection)
        .await?;
    if invalid.identity_groups == 0 {
        return Ok(());
    }
    Err(MigrationError::InvalidOAuthAppIdentities {
        groups: invalid.identity_groups,
        sample: invalid
            .sample
            .unwrap_or_else(|| "sample unavailable".to_owned()),
    })
}

pub(crate) async fn run_ops(pool: &PgPool) -> Result<(), MigrationError> {
    run_set(pool, OPS_LOCK, &OPS).await
}

async fn run_set(
    pool: &PgPool,
    lock: i64,
    migrator: &sqlx::migrate::Migrator,
) -> Result<(), MigrationError> {
    let mut connection = pool.acquire().await?;
    prepare(&mut connection, lock).await?;
    let migration = migrator
        .run(&mut *connection)
        .await
        .map_err(MigrationError::Migration);
    finish(&mut connection, lock, migration).await
}

async fn prepare(connection: &mut sqlx::PgConnection, lock: i64) -> Result<(), MigrationError> {
    sqlx::query_file!("queries/migrations/disable_statement_timeout.sql")
        .execute(&mut *connection)
        .await?;
    sqlx::query_file!("queries/migrations/disable_lock_timeout.sql")
        .execute(&mut *connection)
        .await?;
    sqlx::query_file!("queries/migrations/lock.sql", lock)
        .execute(&mut *connection)
        .await?;
    Ok(())
}

async fn finish(
    connection: &mut sqlx::PgConnection,
    lock: i64,
    migration: Result<(), MigrationError>,
) -> Result<(), MigrationError> {
    let unlock = sqlx::query_file_scalar!("queries/migrations/unlock.sql", lock)
        .fetch_one(&mut *connection)
        .await;

    match (migration, unlock) {
        (Ok(()), Ok(true)) => Ok(()),
        (Err(error), _) => Err(error),
        (Ok(()), Ok(false)) => Err(MigrationError::AdvisoryLockNotHeld),
        (Ok(()), Err(source)) => Err(MigrationError::Database(source)),
    }
}

async fn migrate_environment_connections(
    connection: &mut sqlx::PgConnection,
    bootstrap_app: Option<&OAuthAppBootstrap>,
) -> Result<(), MigrationError> {
    let legacy_column_exists = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (
             SELECT 1
             FROM information_schema.columns
             WHERE table_schema = current_schema()
               AND table_name = 'soundcloud_connections'
               AND column_name = 'uses_environment_oauth_app'
         )",
    )
    .fetch_one(&mut *connection)
    .await?;

    let configured_app_id = match bootstrap_app {
        Some(app) => Some(
            sqlx::query_file_scalar!(
                "queries/oauth_apps/bootstrap.sql",
                app.id(),
                &app.name,
                &app.client_id,
                app.client_secret.expose().as_str(),
                &app.redirect_uri
            )
            .fetch_one(&mut *connection)
            .await?,
        ),
        None => None,
    };

    if !legacy_column_exists {
        return Ok(());
    }
    let legacy_count = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM soundcloud_connections WHERE uses_environment_oauth_app",
    )
    .fetch_one(&mut *connection)
    .await?;
    if legacy_count == 0 {
        return Ok(());
    }
    let app_id = configured_app_id.ok_or(MigrationError::LegacyOAuthAppNotConfigured {
        connections: legacy_count,
    })?;
    sqlx::query(include_str!(
        "../../queries/oauth_apps/map_environment_connections.sql"
    ))
    .bind(app_id)
    .execute(&mut *connection)
    .await?;
    Ok(())
}

fn core_prefix() -> sqlx::migrate::Migrator {
    core_through(CONNECTION_MIGRATION)
}

fn core_through(version: i64) -> sqlx::migrate::Migrator {
    sqlx::migrate::Migrator {
        migrations: Cow::Owned(
            CORE.iter()
                .filter(|migration| migration.version <= version)
                .cloned()
                .collect(),
        ),
        ignore_missing: CORE.ignore_missing,
        locking: CORE.locking,
        no_tx: CORE.no_tx,
    }
}

#[derive(Debug, thiserror::Error)]
pub enum MigrationError {
    #[error("migration database operation failed: {0}")]
    Database(#[from] sqlx::Error),

    #[error("migration failed: {0}")]
    Migration(#[source] sqlx::migrate::MigrateError),

    #[error("migration advisory lock was not held by this connection")]
    AdvisoryLockNotHeld,

    #[error(
        "{connections} legacy SoundCloud connections require SOUNDCLOUD_CLIENT_ID and SOUNDCLOUD_CLIENT_SECRET"
    )]
    LegacyOAuthAppNotConfigured { connections: i64 },

    #[error(
        "{groups} invalid OAuth client identity groups require repair before migration: {sample}"
    )]
    InvalidOAuthAppIdentities { groups: i64, sample: String },
}

#[cfg(test)]
mod tests;
