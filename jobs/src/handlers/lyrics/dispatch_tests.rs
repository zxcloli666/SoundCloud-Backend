use backend_contracts::StoredAudioDispatchPayload;
use backend_contracts::pipeline::TranscriptionRequest;
use sqlx::PgPool;
use url::Url;

use super::{message_id, prepare};

async fn seed_stored_track(pool: &PgPool, generation: i64) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO tracks (
             sc_track_id, urn, title, title_normalized, duration_ms, storage_state
         ) VALUES ('42', 'soundcloud:tracks:42', 'Song', 'song', 180000, 'ok')",
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO storage_event_state (
             sc_track_id, stream, stream_sequence, event_published_at, uploaded_generation
         ) VALUES ('42', 'STORAGE_EVENTS', 1, now(), $1)",
    )
    .bind(generation)
    .execute(pool)
    .await?;
    Ok(())
}

async fn seed_lyrics(pool: &PgPool, plain_text: &str) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO lyrics_cache (sc_track_id, plain_text, source, language)
         VALUES ('42', $1, 'genius', 'EN')",
    )
    .bind(plain_text)
    .execute(pool)
    .await?;
    Ok(())
}

async fn prepared(pool: &PgPool, generation: i64) -> anyhow::Result<Option<TranscriptionRequest>> {
    let storage = Url::parse("https://storage.example/api")?;
    prepare(
        pool,
        &storage,
        StoredAudioDispatchPayload {
            sc_track_id: "42".to_owned(),
            uploaded_generation: generation,
        },
    )
    .await
    .map_err(|error| anyhow::anyhow!("{error}"))
}

async fn wire(pool: &PgPool) -> anyhow::Result<(String, i64, i64, Option<String>)> {
    Ok(sqlx::query_as(
        "SELECT status, upload_generation, attempt, quarantine_reason
         FROM transcription_wire_state WHERE sc_track_id = '42'",
    )
    .fetch_one(pool)
    .await?)
}

#[sqlx::test(migrations = "../api/migrations")]
async fn the_first_dispatch_claims_attempt_one_with_the_whole_reference(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed_stored_track(&pool, 3).await?;
    seed_lyrics(&pool, "first line\n\n[Chorus]\nsecond line\n").await?;

    let request = prepared(&pool, 3).await?;

    let marked = sqlx::query_as::<_, (Option<String>, Option<i64>)>(
        "SELECT track.transcribe_state, storage.transcription_generation
         FROM tracks AS track
         JOIN storage_event_state AS storage USING (sc_track_id)
         WHERE track.sc_track_id = '42'",
    )
    .fetch_one(&pool)
    .await?;
    let request = request.ok_or_else(|| anyhow::anyhow!("dispatch was not prepared"))?;
    assert_eq!(
        request,
        TranscriptionRequest {
            sc_track_id: "42".to_owned(),
            upload_generation: 3,
            attempt: 1,
            audio_url: "https://storage.example/api/redirect/soundcloud_tracks_42.m4a".to_owned(),
            reference_text: "first line\n\n[Chorus]\nsecond line".to_owned(),
            reference_lines_total: 3,
            language: Some("en".to_owned()),
            mode: backend_contracts::pipeline::TranscriptionMode::Align,
        }
    );
    assert_eq!(message_id(&request), "transcribe:42:3:1");
    assert_eq!(wire(&pool).await?, ("pending".to_owned(), 3, 1, None));
    assert_eq!(marked, (Some("pending".to_owned()), Some(3)));
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn the_wire_payload_has_no_initial_prompt(pool: PgPool) -> anyhow::Result<()> {
    seed_stored_track(&pool, 1).await?;
    seed_lyrics(&pool, "only line").await?;

    let request = prepared(&pool, 1)
        .await?
        .ok_or_else(|| anyhow::anyhow!("dispatch was not prepared"))?;

    let payload = serde_json::to_value(&request)?;
    let mut keys = payload
        .as_object()
        .map(|object| object.keys().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    keys.sort();
    assert_eq!(
        keys,
        vec![
            "attempt",
            "audio_url",
            "language",
            "mode",
            "reference_lines_total",
            "reference_text",
            "sc_track_id",
            "upload_generation",
        ]
    );
    assert_eq!(payload["mode"], "align");
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn a_retry_of_the_same_generation_reuses_the_claimed_attempt(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed_stored_track(&pool, 1).await?;
    seed_lyrics(&pool, "only line").await?;

    let first = prepared(&pool, 1).await?;
    let retry = prepared(&pool, 1).await?;
    sqlx::query("UPDATE transcription_wire_state SET attempt = 4 WHERE sc_track_id = '42'")
        .execute(&pool)
        .await?;
    let reopened = prepared(&pool, 1).await?;

    assert!(first.is_some());
    assert_eq!(first, retry);
    assert_eq!(
        reopened.map(|request| message_id(&request)).as_deref(),
        Some("transcribe:42:1:4")
    );
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn a_long_reference_is_cut_between_lines_and_still_counts_every_line(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed_stored_track(&pool, 1).await?;
    let text = vec!["строка текста, которую надо выровнять"; 1_000].join("\n");
    seed_lyrics(&pool, &text).await?;

    let request = prepared(&pool, 1)
        .await?
        .ok_or_else(|| anyhow::anyhow!("dispatch was not prepared"))?;

    assert!(request.reference_text.len() <= 16_000);
    assert!(text.starts_with(&request.reference_text));
    assert!(request.reference_text.ends_with("выровнять"));
    assert_eq!(request.reference_lines_total, 1_000);
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn a_reference_without_a_whole_line_under_the_limit_is_quarantined(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed_stored_track(&pool, 1).await?;
    seed_lyrics(&pool, &"слово ".repeat(3_000)).await?;

    let request = prepared(&pool, 1).await?;

    let track_state: Option<String> =
        sqlx::query_scalar("SELECT transcribe_state FROM tracks WHERE sc_track_id = '42'")
            .fetch_one(&pool)
            .await?;
    assert_eq!(request, None);
    assert_eq!(
        wire(&pool).await?,
        (
            "quarantined".to_owned(),
            1,
            1,
            Some("reference_text_unusable".to_owned())
        )
    );
    assert_eq!(track_state.as_deref(), Some("quarantined"));
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn a_stale_generation_or_synced_lyrics_claim_nothing(pool: PgPool) -> anyhow::Result<()> {
    seed_stored_track(&pool, 2).await?;
    seed_lyrics(&pool, "only line").await?;

    let stale = prepared(&pool, 1).await?;
    sqlx::query("UPDATE lyrics_cache SET synced_lrc = '[00:01.00]only line'")
        .execute(&pool)
        .await?;
    let synced = prepared(&pool, 2).await?;

    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM transcription_wire_state")
        .fetch_one(&pool)
        .await?;
    assert_eq!(stale, None);
    assert_eq!(synced, None);
    assert_eq!(rows, 0);
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn a_finished_attempt_is_not_dispatched_again(pool: PgPool) -> anyhow::Result<()> {
    seed_stored_track(&pool, 1).await?;
    seed_lyrics(&pool, "only line").await?;
    prepared(&pool, 1).await?;
    sqlx::raw_sql(
        "UPDATE transcription_wire_state
         SET status = 'rejected', reason = 'low_confidence', sync_version = 's2.a.b.c',
             completed_at = now()
         WHERE sc_track_id = '42';
         UPDATE tracks SET transcribe_state = 'rejected' WHERE sc_track_id = '42';",
    )
    .execute(&pool)
    .await?;

    assert_eq!(prepared(&pool, 1).await?, None);
    assert_eq!(wire(&pool).await?.0, "rejected");
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn an_unpublished_dispatch_is_released_and_reclaimed_by_its_retry(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed_stored_track(&pool, 1).await?;
    seed_lyrics(&pool, "only line").await?;
    let first = prepared(&pool, 1)
        .await?
        .ok_or_else(|| anyhow::anyhow!("dispatch was not prepared"))?;

    super::abandon(&pool, &first).await;
    let released = sqlx::query_as::<_, (String, Option<String>)>(
        "SELECT status, reason FROM transcription_wire_state WHERE sc_track_id = '42'",
    )
    .fetch_one(&pool)
    .await?;
    let retry = prepared(&pool, 1).await?;

    assert_eq!(
        released,
        (
            "reopenable".to_owned(),
            Some("dispatch_publish_failed".to_owned())
        )
    );
    assert_eq!(retry, Some(first));
    assert_eq!(wire(&pool).await?, ("pending".to_owned(), 1, 1, None));
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn a_new_upload_reclaims_a_transcription_its_predecessor_quarantined(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed_stored_track(&pool, 1).await?;
    seed_lyrics(&pool, "only line").await?;
    prepared(&pool, 1).await?;
    sqlx::raw_sql(
        "UPDATE transcription_wire_state
         SET status = 'quarantined', quarantine_reason = 'new_upload_during_pending',
             completed_at = now(), attempt = 3
         WHERE sc_track_id = '42';
         UPDATE tracks SET transcribe_state = 'quarantined' WHERE sc_track_id = '42';
         UPDATE storage_event_state SET uploaded_generation = 2 WHERE sc_track_id = '42';",
    )
    .execute(&pool)
    .await?;

    let stale = prepared(&pool, 1).await?;
    let current = prepared(&pool, 2).await?;

    let track_state: Option<String> =
        sqlx::query_scalar("SELECT transcribe_state FROM tracks WHERE sc_track_id = '42'")
            .fetch_one(&pool)
            .await?;
    assert_eq!(stale, None);
    assert_eq!(
        current.map(|request| message_id(&request)).as_deref(),
        Some("transcribe:42:2:1")
    );
    assert_eq!(wire(&pool).await?, ("pending".to_owned(), 2, 1, None));
    assert_eq!(track_state.as_deref(), Some("pending"));
    Ok(())
}
