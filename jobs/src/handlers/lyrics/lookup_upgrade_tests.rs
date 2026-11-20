use sqlx::PgPool;

use super::super::LyricsLookupHandler;
use super::{
    PLAIN_BODY, cached_text, candidate, insert_track, lookup_job, offline_handler, recheck_state,
    settle, sweep_job,
};

async fn cache_row(
    pool: &PgPool,
    id: &str,
    source: &str,
    synced: Option<&str>,
    age_days: i32,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO lyrics_cache (
             sc_track_id, synced_lrc, plain_text, source, plain_source, content_generation, updated_at
         ) VALUES ($1, $2, 'plain', $3, $3, 3, now() - $4 * interval '1 day')",
    )
    .bind(id)
    .bind(synced)
    .bind(source)
    .bind(age_days)
    .execute(pool)
    .await?;
    Ok(())
}

async fn queued_upgrades(pool: &PgPool) -> anyhow::Result<Vec<(String, i16)>> {
    Ok(
        sqlx::query_as(
            "SELECT sc_track_id, priority FROM lyrics_lookup_state ORDER BY sc_track_id",
        )
        .fetch_all(pool)
        .await?,
    )
}

fn upgrading(handler: &mut LyricsLookupHandler, batch: i64) {
    handler.config.upgrade_batch = batch;
    handler.config.upgrade_age_days = 60;
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn the_sweep_queues_old_plain_lrclib_rows_up_to_twice_the_batch(
    pool: PgPool,
) -> anyhow::Result<()> {
    for id in ["1", "2", "3", "4", "5", "6", "7"] {
        insert_track(&pool, id, "Lose Yourself").await?;
    }
    sqlx::query("DELETE FROM lyrics_lookup_state WHERE sc_track_id <> '7'")
        .execute(&pool)
        .await?;
    sqlx::query("UPDATE lyrics_lookup_state SET next_run_at = now() - interval '41 days'")
        .execute(&pool)
        .await?;
    cache_row(&pool, "1", "lrclib", None, 90).await?;
    cache_row(&pool, "2", "lrclib", None, 10).await?;
    cache_row(&pool, "3", "lrclib", Some("[00:01.00] line"), 90).await?;
    cache_row(&pool, "4", "genius", None, 90).await?;
    cache_row(&pool, "5", "lrclib", None, 80).await?;
    cache_row(&pool, "6", "lrclib", None, 70).await?;
    let mut handler = offline_handler(&pool)?;
    upgrading(&mut handler, 1);

    handler.enqueue_synced_upgrades().await?;
    assert_eq!(
        queued_upgrades(&pool).await?,
        vec![("1".to_owned(), 5), ("7".to_owned(), 5)]
    );
    handler.enqueue_synced_upgrades().await?;
    handler.enqueue_synced_upgrades().await?;

    let marked: Vec<String> = sqlx::query_scalar(
        "SELECT sc_track_id FROM lyrics_lookup_state WHERE last_outcome = 'upgrade' ORDER BY sc_track_id",
    )
    .fetch_all(&pool)
    .await?;
    assert_eq!(marked, vec!["1".to_owned(), "5".to_owned()]);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_zero_batch_switches_the_upgrade_off(pool: PgPool) -> anyhow::Result<()> {
    insert_track(&pool, "1", "Lose Yourself").await?;
    sqlx::query("DELETE FROM lyrics_lookup_state")
        .execute(&pool)
        .await?;
    cache_row(&pool, "1", "lrclib", None, 90).await?;
    let handler = offline_handler(&pool)?;

    handler.enqueue_synced_upgrades().await?;

    assert!(queued_upgrades(&pool).await?.is_empty());
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn an_upgrade_that_finds_the_same_plain_text_only_marks_the_row_checked(
    pool: PgPool,
) -> anyhow::Result<()> {
    insert_track(&pool, "42", "Lose Yourself").await?;
    sqlx::query("DELETE FROM lyrics_lookup_state")
        .execute(&pool)
        .await?;
    sqlx::query(
        "INSERT INTO lyrics_cache (
             sc_track_id, plain_text, source, plain_source, content_generation, updated_at, embedded_at
         ) VALUES ('42', $1, 'lrclib', 'lrclib', 3, now() - interval '90 days', now() - interval '80 days')",
    )
    .bind(PLAIN_BODY)
    .execute(&pool)
    .await?;
    let mut handler = offline_handler(&pool)?;
    upgrading(&mut handler, 1);
    handler.enqueue_synced_upgrades().await?;
    sqlx::query(
        "UPDATE lyrics_lookup_state SET wake_message_id = gen_random_uuid(), wake_generation = generation",
    )
    .execute(&pool)
    .await?;
    let job = lookup_job(&pool).await?;

    settle(
        &pool,
        &handler,
        &job,
        candidate("lrclib", None, PLAIN_BODY),
        false,
    )
    .await?;

    let (synced, plain, generation) = cached_text(&pool).await?;
    let (checked, embedded): (bool, bool) = sqlx::query_as(
        "SELECT updated_at > now() - interval '1 hour', embedded_at < now() - interval '1 day'
         FROM lyrics_cache WHERE sc_track_id = '42'",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(
        (synced, plain.as_deref(), generation),
        (None, Some(PLAIN_BODY), 3)
    );
    assert!(checked && embedded);
    assert_eq!(recheck_state(&pool).await?, None);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn an_upgrade_that_finds_synced_text_adds_it_and_reopens_the_embedding(
    pool: PgPool,
) -> anyhow::Result<()> {
    insert_track(&pool, "42", "Lose Yourself").await?;
    sqlx::query("DELETE FROM lyrics_lookup_state")
        .execute(&pool)
        .await?;
    cache_row(&pool, "42", "lrclib", None, 90).await?;
    let mut handler = offline_handler(&pool)?;
    upgrading(&mut handler, 1);
    handler.enqueue_synced_upgrades().await?;
    sqlx::query(
        "UPDATE lyrics_lookup_state SET wake_message_id = gen_random_uuid(), wake_generation = generation",
    )
    .execute(&pool)
    .await?;
    let job = lookup_job(&pool).await?;

    let synced = candidate("lrclib", Some("[00:01.00] A synced line"), "A synced line");
    settle(&pool, &handler, &job, synced, false).await?;

    let (synced_lrc, plain, generation) = cached_text(&pool).await?;
    assert_eq!(synced_lrc.as_deref(), Some("[00:01.00] A synced line"));
    assert_eq!(plain.as_deref(), Some("plain"));
    assert_eq!(generation, 4);
    assert_eq!(recheck_state(&pool).await?, None);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn the_sweep_claims_upgrade_rows_under_a_backlog_of_older_rows(
    pool: PgPool,
) -> anyhow::Result<()> {
    for id in 1..=6 {
        insert_track(&pool, &id.to_string(), "Lose Yourself").await?;
    }
    sqlx::query("UPDATE lyrics_lookup_state SET next_run_at = now() - interval '41 days'")
        .execute(&pool)
        .await?;
    sqlx::query(
        "UPDATE lyrics_lookup_state SET priority = 5, last_outcome = 'upgrade',
                next_run_at = now() - interval '1 minute' WHERE sc_track_id = '6'",
    )
    .execute(&pool)
    .await?;
    let job = sweep_job(&pool).await?;

    let claimed = super::super::tiered::claim(
        &pool,
        &job,
        4,
        0,
        1,
        900,
        crate::handlers::star_priority::Turns::default().next(),
    )
    .await?;

    let ids: Vec<String> = claimed
        .into_iter()
        .filter_map(|work| match work {
            super::super::Work::Lookup(claim) => Some(claim.sc_track_id),
            super::super::Work::Refetch(_) => None,
        })
        .collect();
    assert_eq!(ids.first().map(String::as_str), Some("6"));
    assert_eq!(ids.len(), 4);
    Ok(())
}
