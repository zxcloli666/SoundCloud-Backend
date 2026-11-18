use sqlx::{Connection, PgConnection};

use super::{CORE, MigrationError};

struct Rewritten {
    version: i64,
    old_checksum: &'static str,
    drops: &'static [&'static str],
}

const REWRITTEN: &[Rewritten] = &[Rewritten {
    version: 442,
    old_checksum: "98fb174afb743b31b452c44868ef2ba2f49e08fbd0f9ee2e32cb639284966bd0ef01503b3046f250a6c340f425dcafc8",
    drops: &[
        "ALTER TABLE public.track_videos DROP CONSTRAINT IF EXISTS track_videos_track_fk",
        "ALTER TABLE public.track_video_dispatch DROP CONSTRAINT IF EXISTS track_video_dispatch_track_fk",
    ],
}];

pub(super) async fn repair(connection: &mut PgConnection) -> Result<(), MigrationError> {
    for rewritten in REWRITTEN {
        repair_one(connection, rewritten).await?;
    }
    Ok(())
}

async fn repair_one(
    connection: &mut PgConnection,
    rewritten: &Rewritten,
) -> Result<(), MigrationError> {
    let Some(current) = CORE
        .iter()
        .find(|migration| migration.version == rewritten.version)
    else {
        return Ok(());
    };
    let applied_old: bool = sqlx::query_scalar(
        "SELECT EXISTS (
             SELECT 1 FROM _sqlx_migrations
             WHERE version = $1 AND checksum = decode($2, 'hex')
         )",
    )
    .bind(rewritten.version)
    .bind(rewritten.old_checksum)
    .fetch_one(&mut *connection)
    .await?;
    if !applied_old {
        return Ok(());
    }

    let mut tx = connection.begin().await?;
    sqlx::query("SET LOCAL lock_timeout = '5s'")
        .execute(&mut *tx)
        .await?;
    for drop in rewritten.drops {
        sqlx::query(*drop).execute(&mut *tx).await?;
    }
    sqlx::query("UPDATE _sqlx_migrations SET checksum = $1 WHERE version = $2")
        .bind(current.checksum.as_ref())
        .bind(rewritten.version)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}
