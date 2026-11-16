use std::str::FromStr;

use anyhow::Context;
use tracing::info;

use crate::config::{DatabaseConfig, OAuthAppBootstrap, ScdNode};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MigrationScope {
    Core,
    Ops,
    All,
}

impl FromStr for MigrationScope {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "core" => Ok(Self::Core),
            "ops" => Ok(Self::Ops),
            "all" => Ok(Self::All),
            _ => Err(anyhow::anyhow!("migration scope must be core, ops, or all")),
        }
    }
}

pub async fn run(scope: MigrationScope, core_target: Option<i64>) -> anyhow::Result<()> {
    if core_target.is_some() && scope != MigrationScope::Core {
        anyhow::bail!("a migration target applies to the core scope only");
    }
    match scope {
        MigrationScope::Core => migrate_core(core_target).await?,
        MigrationScope::Ops => migrate_ops().await?,
        MigrationScope::All => {
            migrate_core(None).await?;
            migrate_ops().await?;
        }
    }
    Ok(())
}

async fn migrate_core(target: Option<i64>) -> anyhow::Result<()> {
    let database = DatabaseConfig::from_env("", true, "36")
        .context("core migration database configuration is invalid")?;
    let bootstrap_app =
        OAuthAppBootstrap::from_env().context("core migration OAuth configuration is invalid")?;
    let scd_node = ScdNode::from_env().context("SCD_NODE, SCD_ROLE and SCD_MODE are invalid")?;
    let pool = crate::db::connect_migration(&database, "scd-jobs-migrate:core")
        .await
        .context("core migration database connection failed")?;
    let result = migrate_core_on(&pool, bootstrap_app.as_ref(), scd_node.as_ref(), target).await;
    pool.close().await;
    result
}

async fn migrate_core_on(
    pool: &sqlx::PgPool,
    bootstrap_app: Option<&OAuthAppBootstrap>,
    scd_node: Option<&ScdNode>,
    target: Option<i64>,
) -> anyhow::Result<()> {
    crate::db::migrations::run_core_up_to(pool, bootstrap_app, target)
        .await
        .context("core migrations failed")?;
    info!(target = ?target, "core migrations applied");
    let Some(node) = scd_node else {
        return Ok(());
    };
    let changes = crate::db::migrations::configure_node(pool, node)
        .await
        .context("scd node configuration failed")?;
    info!(
        node = %node.node,
        role = %node.role,
        mode = %node.mode,
        changes = ?changes,
        "scd node configured"
    );
    Ok(())
}

async fn migrate_ops() -> anyhow::Result<()> {
    let database = DatabaseConfig::from_env("OPS_", false, "4")
        .context("ops migration database configuration is invalid")?;
    let pool = crate::db::connect_migration(&database, "scd-jobs-migrate:ops")
        .await
        .context("ops migration database connection failed")?;
    let result = crate::db::migrations::run_ops(&pool)
        .await
        .context("ops migrations failed");
    pool.close().await;
    result?;
    info!("ops migrations applied");
    Ok(())
}
