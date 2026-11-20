use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use super::{
    RECONCILABLE, ensure_catalog, install_schema, journal_operation, membership_mutations,
    membership_of, projection_of, reduce_with, seed_reconcilable_playlist, snapshot_of,
};

const SOUNDCLOUD: [&str; 3] = ["1", "2", "7"];
const REBASED: [&str; 2] = ["1", "7"];

async fn seed_legacy_playlist(pool: &PgPool, edited: bool) -> anyhow::Result<()> {
    install_schema(pool).await?;
    ensure_catalog(pool, &["1", "2", "5", "7", "9"]).await?;
    let observation_id = seed_reconcilable_playlist(pool, &["1", "2", "5", "9"]).await?;
    sqlx::query(
        "INSERT INTO playlist_legacy_membership_intents (
             archive_id, queue_id, source, playlist_urn, legacy_desired_revision,
             legacy_synced_revision, queue_created_at, classification
         ) VALUES ($1, $1, 'queue', $2, 2, 1, now() - interval '1 day', 'membership_diverged')",
    )
    .bind(Uuid::now_v7())
    .bind(RECONCILABLE)
    .execute(pool)
    .await?;
    if !edited {
        return Ok(());
    }
    journal_operation(pool, observation_id, 1, "remove", Some("2"), None, None).await?;
    sqlx::query(
        "DELETE FROM playlist_track_projection WHERE playlist_urn = $1 AND sc_track_id = '2'",
    )
    .bind(RECONCILABLE)
    .execute(pool)
    .await?;
    Ok(())
}

async fn soundcloud_had(pool: &PgPool, track_ids: &[&str], days_ago: i32) -> anyhow::Result<()> {
    let snapshot_id: Uuid = sqlx::query_scalar(
        "INSERT INTO playlist_remote_snapshots (playlist_urn, content_fingerprint, track_count)
         VALUES ($1, sha256($2::bytea), $3) RETURNING id",
    )
    .bind(RECONCILABLE)
    .bind(format!("earlier:{}", track_ids.join(",")).into_bytes())
    .bind(i32::try_from(track_ids.len())?)
    .fetch_one(pool)
    .await?;
    for (position, sc_track_id) in track_ids.iter().enumerate() {
        sqlx::query(
            "INSERT INTO playlist_remote_snapshot_tracks (snapshot_id, position, sc_track_id)
             VALUES ($1, $2, $3)",
        )
        .bind(snapshot_id)
        .bind(i32::try_from(position)?)
        .bind(sc_track_id)
        .execute(pool)
        .await?;
    }
    sqlx::query(
        "UPDATE playlist_remote_observations
         SET snapshot_id = $2,
             declared_track_count = $3,
             observed_track_count = $3,
             sc_last_modified = now() - make_interval(days => $4),
             observed_at = now() - make_interval(days => $4)
         WHERE playlist_urn = $1",
    )
    .bind(RECONCILABLE)
    .bind(snapshot_id)
    .bind(i32::try_from(track_ids.len())?)
    .bind(days_ago)
    .execute(pool)
    .await?;
    Ok(())
}

async fn pending_additions(pool: &PgPool) -> anyhow::Result<Vec<String>> {
    Ok(sqlx::query_scalar(
        "SELECT track_id FROM playlist_membership_operations
         WHERE playlist_urn = $1 AND kind = 'add' AND outcome = 'pending' ORDER BY sequence",
    )
    .bind(RECONCILABLE)
    .fetch_all(pool)
    .await?)
}

async fn intents(pool: &PgPool) -> anyhow::Result<Vec<(String, Option<String>, bool)>> {
    Ok(sqlx::query_as(
        "SELECT classification, prior_classification, local_snapshot_id IS NOT NULL
         FROM playlist_legacy_membership_intents WHERE playlist_urn = $1",
    )
    .bind(RECONCILABLE)
    .fetch_all(pool)
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
async fn a_legacy_playlist_edited_on_soundcloud_later_takes_the_soundcloud_tracks_and_keeps_the_old_ones_on_record(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed_legacy_playlist(&pool, false).await?;
    soundcloud_had(&pool, &["1", "2"], 0).await?;

    reduce_with(&pool, &snapshot_of(&SOUNDCLOUD), true).await?;

    assert_eq!(projection_of(&pool).await?, SOUNDCLOUD);
    assert_eq!(membership_of(&pool).await?.0, "clean");
    assert!(pushed_tracks(&pool).await?.is_empty());
    assert!(pending_additions(&pool).await?.is_empty());
    assert_eq!(
        intents(&pool).await?,
        [(
            "abandoned".to_owned(),
            Some("membership_diverged".to_owned()),
            true
        )]
    );
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn an_unpushed_edit_of_a_legacy_playlist_is_rebased_onto_soundcloud_and_sent(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed_legacy_playlist(&pool, true).await?;

    reduce_with(&pool, &snapshot_of(&SOUNDCLOUD), true).await?;

    assert_eq!(projection_of(&pool).await?, REBASED);
    assert_eq!(membership_of(&pool).await?.0, "shadow_ready");
    assert_eq!(pushed_tracks(&pool).await?, [json!(REBASED)]);
    assert_eq!(intents(&pool).await?[0].0, "abandoned");
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn the_rebased_edit_commits_once_soundcloud_has_it(pool: PgPool) -> anyhow::Result<()> {
    seed_legacy_playlist(&pool, true).await?;
    reduce_with(&pool, &snapshot_of(&SOUNDCLOUD), true).await?;
    assert_eq!(operation_sequences(&pool).await?, (0, 1));

    reduce_with(&pool, &snapshot_of(&REBASED), true).await?;

    assert_eq!(projection_of(&pool).await?, REBASED);
    assert_eq!(membership_of(&pool).await?.0, "clean");
    assert_eq!(operation_sequences(&pool).await?, (1, 1));
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn the_unpushed_additions_of_a_legacy_playlist_go_onto_soundcloud_in_their_place(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed_legacy_playlist(&pool, false).await?;
    soundcloud_had(&pool, &["1", "2"], 2).await?;

    reduce_with(&pool, &snapshot_of(&SOUNDCLOUD), true).await?;

    let kept = ["1", "2", "5", "9", "7"];
    assert_eq!(projection_of(&pool).await?, kept);
    assert_eq!(membership_of(&pool).await?.0, "shadow_ready");
    assert_eq!(pending_additions(&pool).await?, ["5", "9"]);
    assert_eq!(pushed_tracks(&pool).await?, [json!(kept)]);
    assert_eq!(intents(&pool).await?[0].0, "abandoned");

    reduce_with(&pool, &snapshot_of(&kept), true).await?;

    assert_eq!(membership_of(&pool).await?.0, "clean");
    assert!(pending_additions(&pool).await?.is_empty());
    assert_eq!(operation_sequences(&pool).await?, (2, 2));
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_legacy_track_soundcloud_once_had_leaves_with_soundcloud(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed_legacy_playlist(&pool, false).await?;
    soundcloud_had(&pool, &["1", "2", "9"], 2).await?;

    reduce_with(&pool, &snapshot_of(&SOUNDCLOUD), true).await?;

    assert_eq!(projection_of(&pool).await?, ["1", "2", "5", "7"]);
    assert_eq!(pending_additions(&pool).await?, ["5"]);
    Ok(())
}
