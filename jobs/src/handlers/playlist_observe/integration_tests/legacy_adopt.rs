use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use super::{
    RECONCILABLE, ensure_catalog, install_schema, journal_operation, membership_mutations,
    membership_of, projection_of, reduce_with, seed_reconcilable_playlist, snapshot_of,
};

const EDITED: [&str; 4] = ["1", "5", "9", "7"];

async fn seed_edited_legacy_playlist(pool: &PgPool) -> anyhow::Result<()> {
    install_schema(pool).await?;
    ensure_catalog(pool, &["1", "2", "5", "7", "9"]).await?;
    let observation_id = seed_reconcilable_playlist(pool, &["1", "2", "5", "9"]).await?;
    sqlx::query(
        "INSERT INTO playlist_legacy_membership_intents (
             archive_id, source, playlist_urn,
             legacy_desired_revision, legacy_synced_revision, classification
         ) VALUES ($1, 'revision', $2, 2, 1, 'membership_diverged')",
    )
    .bind(Uuid::now_v7())
    .bind(RECONCILABLE)
    .execute(pool)
    .await?;
    journal_operation(pool, observation_id, 1, "remove", Some("2"), None, None).await?;
    sqlx::query(
        "DELETE FROM playlist_track_projection WHERE playlist_urn = $1 AND sc_track_id = '2'",
    )
    .bind(RECONCILABLE)
    .execute(pool)
    .await?;
    Ok(())
}

async fn open_intents(pool: &PgPool) -> anyhow::Result<i64> {
    Ok(sqlx::query_scalar(
        "SELECT count(*) FROM playlist_legacy_membership_intents
         WHERE playlist_urn = $1 AND resolved_at IS NULL",
    )
    .bind(RECONCILABLE)
    .fetch_one(pool)
    .await?)
}

async fn pushed_tracks(pool: &PgPool) -> anyhow::Result<Vec<Value>> {
    Ok(membership_mutations(pool)
        .await?
        .into_iter()
        .map(|(_, payload)| payload["tracks"].clone())
        .collect())
}

async fn operation_sequences(pool: &PgPool) -> anyhow::Result<(i64, i64)> {
    Ok(sqlx::query_as(
        "SELECT committed_operation_sequence, last_operation_sequence
         FROM playlist_membership_state WHERE playlist_urn = $1",
    )
    .bind(RECONCILABLE)
    .fetch_one(pool)
    .await?)
}

#[sqlx::test(migrations = false)]
async fn an_edited_legacy_playlist_keeps_its_local_tracks_and_is_sent_to_soundcloud(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed_edited_legacy_playlist(&pool).await?;

    reduce_with(&pool, &snapshot_of(&["1", "2", "7"]), true).await?;

    assert_eq!(projection_of(&pool).await?, EDITED);
    assert_eq!(membership_of(&pool).await?.0, "shadow_ready");
    assert_eq!(pushed_tracks(&pool).await?, [json!(EDITED)]);
    assert_eq!(open_intents(&pool).await?, 1);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_second_read_before_soundcloud_takes_the_write_keeps_the_legacy_tracks(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed_edited_legacy_playlist(&pool).await?;
    let remote = snapshot_of(&["1", "2", "7"]);

    reduce_with(&pool, &remote, true).await?;
    reduce_with(&pool, &remote, true).await?;

    assert_eq!(projection_of(&pool).await?, EDITED);
    assert_eq!(pushed_tracks(&pool).await?, [json!(EDITED)]);
    assert_eq!(open_intents(&pool).await?, 1);
    assert_eq!(operation_sequences(&pool).await?, (0, 1));
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn the_legacy_intents_close_once_soundcloud_has_the_tracks_and_the_edit_then_commits(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed_edited_legacy_playlist(&pool).await?;
    reduce_with(&pool, &snapshot_of(&["1", "2", "7"]), true).await?;

    reduce_with(&pool, &snapshot_of(&EDITED), true).await?;

    assert_eq!(open_intents(&pool).await?, 0);
    assert_eq!(projection_of(&pool).await?, EDITED);

    reduce_with(&pool, &snapshot_of(&EDITED), true).await?;

    assert_eq!(projection_of(&pool).await?, EDITED);
    assert_eq!(operation_sequences(&pool).await?, (1, 1));
    Ok(())
}
