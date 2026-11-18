use sqlx::PgPool;

use super::*;

const OLD_442_CHECKSUM: &str = "98fb174afb743b31b452c44868ef2ba2f49e08fbd0f9ee2e32cb639284966bd0ef01503b3046f250a6c340f425dcafc8";

#[sqlx::test(migrations = false)]
async fn a_database_that_applied_the_old_video_tables_loses_its_foreign_keys(
    pool: PgPool,
) -> anyhow::Result<()> {
    run_core(&pool, None).await?;
    assert_eq!(video_foreign_keys(&pool).await?, 0);
    sqlx::raw_sql(
        "ALTER TABLE public.track_videos ADD CONSTRAINT track_videos_track_fk
             FOREIGN KEY (sc_track_id) REFERENCES public.tracks(sc_track_id) ON DELETE CASCADE NOT VALID;
         ALTER TABLE public.track_video_dispatch ADD CONSTRAINT track_video_dispatch_track_fk
             FOREIGN KEY (sc_track_id) REFERENCES public.tracks(sc_track_id) ON DELETE CASCADE NOT VALID",
    )
    .execute(&pool)
    .await?;
    sqlx::query("UPDATE _sqlx_migrations SET checksum = decode($1, 'hex') WHERE version = 442")
        .bind(OLD_442_CHECKSUM)
        .execute(&pool)
        .await?;
    assert_eq!(video_foreign_keys(&pool).await?, 2);

    run_core(&pool, None).await?;

    assert_eq!(video_foreign_keys(&pool).await?, 0);
    let repaired: bool = sqlx::query_scalar(
        "SELECT checksum <> decode($1, 'hex') FROM _sqlx_migrations WHERE version = 442",
    )
    .bind(OLD_442_CHECKSUM)
    .fetch_one(&pool)
    .await?;
    assert!(repaired);
    run_core(&pool, None).await?;
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_fresh_database_never_creates_the_video_foreign_keys(pool: PgPool) -> anyhow::Result<()> {
    run_core(&pool, None).await?;
    assert_eq!(video_foreign_keys(&pool).await?, 0);
    Ok(())
}

async fn video_foreign_keys(pool: &PgPool) -> anyhow::Result<i64> {
    Ok(sqlx::query_scalar(
        "SELECT count(*) FROM pg_constraint
         WHERE contype = 'f' AND conrelid IN ('public.track_videos'::regclass, 'public.track_video_dispatch'::regclass)",
    )
    .fetch_one(pool)
    .await?)
}
