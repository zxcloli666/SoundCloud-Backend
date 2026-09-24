use sqlx::PgPool;
use uuid::Uuid;

use super::playlist_legacy::PlaylistLegacyHandler;
use crate::config::PlaylistReconcileConfig;

const PLAYLIST: &str = "soundcloud:playlists:77";

fn handler(pool: &PgPool) -> PlaylistLegacyHandler {
    PlaylistLegacyHandler::new(
        pool.clone(),
        PlaylistReconcileConfig {
            sweep_batch: 128,
            sweep_owner_share: 8,
            claim_seconds: 300,
            legacy_drain_batch: 500,
            membership_remote_apply: false,
        },
    )
}

async fn install(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::raw_sql(
        "CREATE TABLE playlists (urn text PRIMARY KEY);
         CREATE TABLE playlist_membership_state (
             playlist_urn text PRIMARY KEY,
             sync_status text NOT NULL DEFAULT 'legacy_review',
             next_reconcile_at timestamptz,
             updated_at timestamptz NOT NULL DEFAULT now()
         );
         CREATE TABLE playlist_legacy_membership_intents (
             archive_id uuid PRIMARY KEY,
             playlist_urn text NOT NULL,
             classification text NOT NULL DEFAULT 'unclassified',
             prior_classification text,
             archived_at timestamptz NOT NULL DEFAULT now(),
             resolved_at timestamptz,
             CONSTRAINT resolution_complete
                 CHECK ((classification IN ('resolved', 'abandoned')) = (resolved_at IS NOT NULL)),
             CONSTRAINT resolution_provenance
                 CHECK (
                     classification NOT IN ('resolved', 'abandoned')
                     OR prior_classification IS NOT NULL
                 )
         );",
    )
    .execute(pool)
    .await?;
    Ok(())
}

async fn seed(pool: &PgPool, playlist_urn: &str, classifications: &[&str]) -> anyhow::Result<()> {
    sqlx::query("INSERT INTO playlists VALUES ($1) ON CONFLICT DO NOTHING")
        .bind(playlist_urn)
        .execute(pool)
        .await?;
    sqlx::query(
        "INSERT INTO playlist_membership_state (playlist_urn, next_reconcile_at)
         VALUES ($1, clock_timestamp() + interval '1 hour')
         ON CONFLICT DO NOTHING",
    )
    .bind(playlist_urn)
    .execute(pool)
    .await?;
    for classification in classifications {
        sqlx::query(
            "INSERT INTO playlist_legacy_membership_intents (
                 archive_id, playlist_urn, classification
             ) VALUES ($1, $2, $3)",
        )
        .bind(Uuid::now_v7())
        .bind(playlist_urn)
        .bind(classification)
        .execute(pool)
        .await?;
    }
    Ok(())
}

async fn states(
    pool: &PgPool,
    playlist_urn: &str,
) -> anyhow::Result<Vec<(String, Option<String>)>> {
    let rows = sqlx::query_as::<_, (String, Option<String>)>(
        "SELECT classification, prior_classification
         FROM playlist_legacy_membership_intents
         WHERE playlist_urn = $1
         ORDER BY classification, archive_id",
    )
    .bind(playlist_urn)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

async fn is_due(pool: &PgPool, playlist_urn: &str) -> anyhow::Result<bool> {
    Ok(sqlx::query_scalar::<_, bool>(
        "SELECT next_reconcile_at <= clock_timestamp()
         FROM playlist_membership_state WHERE playlist_urn = $1",
    )
    .bind(playlist_urn)
    .fetch_one(pool)
    .await?)
}

#[sqlx::test(migrations = false)]
async fn converged_intents_are_resolved_and_keep_their_diagnosis(
    pool: PgPool,
) -> anyhow::Result<()> {
    install(&pool).await?;
    seed(&pool, PLAYLIST, &["equal", "remote_superset"]).await?;

    handler(&pool).drain().await?;

    assert_eq!(
        states(&pool, PLAYLIST).await?,
        vec![
            ("resolved".to_owned(), Some("equal".to_owned())),
            ("resolved".to_owned(), Some("remote_superset".to_owned())),
        ]
    );
    assert!(is_due(&pool, PLAYLIST).await?);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn an_order_only_intent_is_abandoned_rather_than_claimed_as_resolved(
    pool: PgPool,
) -> anyhow::Result<()> {
    install(&pool).await?;
    seed(&pool, PLAYLIST, &["order_only"]).await?;

    handler(&pool).drain().await?;

    assert_eq!(
        states(&pool, PLAYLIST).await?,
        vec![("abandoned".to_owned(), Some("order_only".to_owned()))]
    );
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_playlist_that_still_holds_a_dangerous_intent_stays_blocked(
    pool: PgPool,
) -> anyhow::Result<()> {
    install(&pool).await?;
    seed(&pool, PLAYLIST, &["equal", "local_superset"]).await?;

    handler(&pool).drain().await?;

    assert_eq!(
        states(&pool, PLAYLIST).await?,
        vec![
            ("local_superset".to_owned(), None),
            ("resolved".to_owned(), Some("equal".to_owned())),
        ]
    );
    assert!(!is_due(&pool, PLAYLIST).await?);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_diverged_intent_is_never_drained_automatically(pool: PgPool) -> anyhow::Result<()> {
    install(&pool).await?;
    seed(&pool, PLAYLIST, &["membership_diverged"]).await?;

    handler(&pool).drain().await?;

    assert_eq!(
        states(&pool, PLAYLIST).await?,
        vec![("membership_diverged".to_owned(), None)]
    );
    assert!(!is_due(&pool, PLAYLIST).await?);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn an_unclassified_intent_makes_its_playlist_due_for_observation(
    pool: PgPool,
) -> anyhow::Result<()> {
    install(&pool).await?;
    seed(&pool, PLAYLIST, &["unclassified"]).await?;

    handler(&pool).drain().await?;

    assert_eq!(
        states(&pool, PLAYLIST).await?,
        vec![("unclassified".to_owned(), None)]
    );
    assert!(is_due(&pool, PLAYLIST).await?);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn draining_is_idempotent(pool: PgPool) -> anyhow::Result<()> {
    install(&pool).await?;
    seed(&pool, PLAYLIST, &["equal"]).await?;

    handler(&pool).drain().await?;
    handler(&pool).drain().await?;

    assert_eq!(
        states(&pool, PLAYLIST).await?,
        vec![("resolved".to_owned(), Some("equal".to_owned()))]
    );
    Ok(())
}
