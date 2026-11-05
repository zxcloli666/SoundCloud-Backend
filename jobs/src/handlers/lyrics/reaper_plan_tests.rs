use serde_json::Value;
use sqlx::PgPool;

const FILLER: i64 = 10_000;
const ROOM: i64 = 10;

async fn seed(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::raw_sql(&format!(
        "INSERT INTO tracks (sc_track_id, urn, title, title_normalized, duration_ms, storage_state)
         SELECT n::text, 'soundcloud:tracks:' || n, 'Song', 'song', 180000, 'ok'
         FROM generate_series(1, {FILLER}) AS n;
         INSERT INTO storage_event_state (
             sc_track_id, stream, stream_sequence, event_published_at,
             uploaded_generation, transcription_generation, updated_at
         )
         SELECT n::text, 'STORAGE_EVENTS', n, now(), 1,
                CASE WHEN n % 5 = 2 THEN 1 END,
                now() - interval '2 hours' - n * interval '1 second'
         FROM generate_series(1, {FILLER}) AS n;
         INSERT INTO lyrics_cache (sc_track_id, plain_text, synced_lrc, source, created_at)
         SELECT n::text, 'plain lyrics',
                CASE WHEN n % 5 = 1 THEN '[00:01.00] synced' END,
                'genius', (now() AT TIME ZONE 'UTC') - interval '1 hour'
         FROM generate_series(1, {FILLER}) AS n
         WHERE n % 5 <> 4;
         INSERT INTO transcription_wire_state (
             sc_track_id, status, upload_generation, dispatched_at
         )
         SELECT n::text, CASE WHEN n % 5 = 2 THEN 'done' ELSE 'pending' END,
                1, now() - interval '1 day'
         FROM generate_series(1, {FILLER}) AS n
         WHERE n % 5 IN (2, 3);"
    ))
    .execute(pool)
    .await?;
    sqlx::raw_sql(
        "INSERT INTO tracks (
             sc_track_id, urn, title, title_normalized, duration_ms, storage_state, transcribe_state
         ) VALUES
             ('reupload', 'soundcloud:tracks:reupload', 'Song', 'song', 180000, 'ok', 'quarantined'),
             ('timeout', 'soundcloud:tracks:timeout', 'Song', 'song', 180000, 'ok', 'quarantined');
         INSERT INTO storage_event_state (
             sc_track_id, stream, stream_sequence, event_published_at,
             uploaded_generation, transcription_generation, updated_at
         ) VALUES
             ('reupload', 'STORAGE_EVENTS', 90001, now(), 2, 1, now() - interval '3 days'),
             ('timeout', 'STORAGE_EVENTS', 90002, now(), 2, 1, now() - interval '4 days');
         INSERT INTO lyrics_cache (sc_track_id, plain_text, source, created_at)
         VALUES ('reupload', 'plain lyrics', 'genius', (now() AT TIME ZONE 'UTC') - interval '1 hour'),
                ('timeout', 'plain lyrics', 'genius', (now() AT TIME ZONE 'UTC') - interval '1 hour');
         INSERT INTO transcription_wire_state (
             sc_track_id, status, upload_generation, dispatched_at, completed_at, quarantine_reason
         ) VALUES
             ('reupload', 'quarantined', 1, now() - interval '1 day', now() - interval '1 hour',
              'reopen_superseded'),
             ('timeout', 'quarantined', 1, now() - interval '1 day', now() - interval '1 hour',
              'result_timeout');
         ANALYZE tracks, storage_event_state, lyrics_cache, transcription_wire_state",
    )
    .execute(pool)
    .await?;
    Ok(())
}

fn nodes<'a>(plan: &'a Value, out: &mut Vec<&'a Value>) {
    match plan {
        Value::Object(map) => {
            if map.contains_key("Node Type") {
                out.push(plan);
            }
            map.values().for_each(|child| nodes(child, out));
        }
        Value::Array(items) => items.iter().for_each(|item| nodes(item, out)),
        _ => {}
    }
}

fn relation_scans<'a>(plan: &'a Value, relation: &str) -> Vec<&'a Value> {
    let mut all = Vec::new();
    nodes(plan, &mut all);
    all.into_iter()
        .filter(|node| node["Relation Name"] == relation)
        .collect()
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn the_alignment_scan_picks_the_oldest_uploads_and_reads_tracks_only_for_them(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed(&pool).await?;
    let sql = include_str!("../../../queries/lyrics/reap_transcriptions_align.sql");

    let picked = sqlx::query_file!(
        "queries/lyrics/reap_transcriptions_align.sql",
        ROOM,
        &Vec::<String>::new()
    )
    .fetch_all(&pool)
    .await?
    .into_iter()
    .map(|candidate| candidate.sc_track_id)
    .collect::<Vec<_>>();

    let mut expected = vec!["reupload".to_owned()];
    expected.extend(
        (1..=FILLER)
            .rev()
            .filter(|n| n % 5 == 0)
            .take(usize::try_from(ROOM)? - 1)
            .map(|n| n.to_string()),
    );
    assert_eq!(picked, expected);

    let mut transaction = pool.begin().await?;
    let plan: Value = sqlx::query_scalar(&format!("EXPLAIN (ANALYZE, FORMAT JSON) {sql}"))
        .bind(ROOM)
        .bind(Vec::<String>::new())
        .fetch_one(&mut *transaction)
        .await?;
    for table in ["tracks", "storage_event_state", "lyrics_cache"] {
        let seq = relation_scans(&plan, table)
            .into_iter()
            .any(|node| node["Node Type"] == "Seq Scan");
        assert!(!seq, "{table} is read end to end: {plan}");
    }
    let track_probes: i64 = relation_scans(&plan, "tracks")
        .iter()
        .map(|node| node["Actual Loops"].as_i64().unwrap_or(i64::MAX))
        .sum();
    assert!(
        track_probes <= 4 * ROOM,
        "tracks must be probed for the picked rows only, not for every candidate: {plan}"
    );
    Ok(())
}

const STALE: i64 = 5_000;
const FRESH: i64 = 3_000;

async fn seed_requests(pool: &PgPool, stale: i64, fresh: i64) -> anyhow::Result<i64> {
    let window = super::result_window(&backend_contracts::worker_contract::LYRICS_LANE)?;
    sqlx::query(
        "INSERT INTO lyrics_cache (sc_track_id, plain_text, source, created_at, embedding_state)
         SELECT 's' || n, 'plain lyrics long enough to be embedded', 'genius',
                (now() AT TIME ZONE 'UTC') - interval '3 days', 'pending'
         FROM generate_series(1, $1::bigint) AS n",
    )
    .bind(stale)
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO lyrics_embedding_wire_state (
             sc_track_id, status, lyrics_created_at, lyrics_content_generation,
             request_version, request_text, request_sha256, request_message_id,
             first_publish_attempt_at, publish_retry_until
         )
         SELECT request.sc_track_id, 'pending', (now() AT TIME ZONE 'UTC') - interval '3 days', 1,
                1, 'plain lyrics', sha256(request.sc_track_id::bytea), 'lyr:' || request.sc_track_id,
                request.at, request.at + interval '1 day'
         FROM (
             SELECT 's' || n AS sc_track_id,
                    now() - ($1::bigint + 3600 + n) * interval '1 second' AS at
             FROM generate_series(1, $2::bigint) AS n
             UNION ALL
             SELECT 'f' || n, now() - n * interval '1 second'
             FROM generate_series(1, $3::bigint) AS n
         ) AS request",
    )
    .bind(window)
    .bind(stale)
    .bind(fresh)
    .execute(pool)
    .await?;
    sqlx::query(
        "UPDATE lyrics_embedding_wire_state AS wire
         SET lyrics_created_at = cache.created_at
         FROM lyrics_cache AS cache
         WHERE cache.sc_track_id = wire.sc_track_id",
    )
    .execute(pool)
    .await?;
    sqlx::query("VACUUM ANALYZE lyrics_cache, lyrics_embedding_wire_state")
        .execute(pool)
        .await?;
    Ok(window)
}

async fn add_orphan(pool: &PgPool, id: &str, waited: i64) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO lyrics_embedding_wire_state (
             sc_track_id, status, lyrics_created_at, lyrics_content_generation,
             request_version, request_text, request_sha256, request_message_id,
             first_publish_attempt_at, publish_retry_until
         )
         VALUES ($1, 'pending', now() AT TIME ZONE 'UTC', 1, 1, 'plain lyrics',
                 sha256($1::bytea), 'lyr:' || $1,
                 now() - $2::bigint * interval '1 second',
                 now() - $2::bigint * interval '1 second' + interval '1 day')",
    )
    .bind(id)
    .bind(waited)
    .execute(pool)
    .await?;
    Ok(())
}

async fn wire_status(pool: &PgPool, id: &str) -> anyhow::Result<String> {
    Ok(
        sqlx::query_scalar("SELECT status FROM lyrics_embedding_wire_state WHERE sc_track_id = $1")
            .bind(id)
            .fetch_one(pool)
            .await?,
    )
}

fn index_rows(plan: &Value, index: &str) -> i64 {
    let mut all = Vec::new();
    nodes(plan, &mut all);
    all.into_iter()
        .filter(|node| node["Index Name"] == index)
        .map(|node| {
            node["Actual Rows"].as_i64().unwrap_or(i64::MAX)
                * node["Actual Loops"].as_i64().unwrap_or(i64::MAX)
        })
        .sum()
}

fn reads_whole(plan: &Value, relation: &str) -> bool {
    relation_scans(plan, relation)
        .into_iter()
        .any(|node| node["Node Type"] == "Seq Scan")
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn the_orphan_scan_reads_only_a_window_of_the_oldest_timed_out_requests(
    pool: PgPool,
) -> anyhow::Result<()> {
    let window = seed_requests(&pool, STALE, FRESH).await?;
    add_orphan(&pool, "orphan-oldest", window + 3600 + STALE + 60).await?;
    add_orphan(&pool, "orphan-newest", window + 60).await?;
    let sql = include_str!("../../../queries/lyrics/quarantine_orphaned_embedding_requests.sql");

    let quarantined = sqlx::query_file_scalar!(
        "queries/lyrics/quarantine_orphaned_embedding_requests.sql",
        super::QUARANTINE_BATCH,
        window,
        super::ORPHAN_SCAN_WINDOW
    )
    .fetch_one(&pool)
    .await?;

    assert_eq!(quarantined, 1);
    assert_eq!(wire_status(&pool, "orphan-oldest").await?, "quarantined");
    assert_eq!(wire_status(&pool, "orphan-newest").await?, "pending");
    let mut transaction = pool.begin().await?;
    let plan: Value = sqlx::query_scalar(&format!("EXPLAIN (ANALYZE, FORMAT JSON) {sql}"))
        .bind(super::QUARANTINE_BATCH)
        .bind(window)
        .bind(super::ORPHAN_SCAN_WINDOW)
        .fetch_one(&mut *transaction)
        .await?;
    transaction.rollback().await?;
    assert!(
        !reads_whole(&plan, "lyrics_embedding_wire_state"),
        "requests are read end to end: {plan}"
    );
    assert!(
        index_rows(&plan, "lyrics_embedding_pending_idx") <= super::ORPHAN_SCAN_WINDOW,
        "the scan walks past its window of the oldest requests: {plan}"
    );
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn the_stale_scan_takes_its_batch_and_stops_at_the_result_window(
    pool: PgPool,
) -> anyhow::Result<()> {
    let stale = super::STALE_EMBEDDING_BATCH + 100;
    let window = seed_requests(&pool, stale, FRESH).await?;
    let sql = include_str!("../../../queries/lyrics/quarantine_stale_embeddings.sql");

    let mut timed_out = Vec::new();
    for _ in 0..2 {
        let round = sqlx::query_file!(
            "queries/lyrics/quarantine_stale_embeddings.sql",
            super::STALE_EMBEDDING_BATCH,
            window,
            super::REPEATED_RESULT_TIMEOUTS
        )
        .fetch_one(&pool)
        .await?;
        timed_out.push(round.timed_out);
    }

    assert_eq!(timed_out, vec![super::STALE_EMBEDDING_BATCH, 100]);
    let released: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM lyrics_cache WHERE embedding_state IS NULL AND sc_track_id LIKE 's%'",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(released, stale);
    let mut transaction = pool.begin().await?;
    let plan: Value = sqlx::query_scalar(&format!("EXPLAIN (ANALYZE, FORMAT JSON) {sql}"))
        .bind(super::STALE_EMBEDDING_BATCH)
        .bind(window)
        .bind(super::REPEATED_RESULT_TIMEOUTS)
        .fetch_one(&mut *transaction)
        .await?;
    transaction.rollback().await?;
    assert!(
        !reads_whole(&plan, "lyrics_embedding_wire_state"),
        "requests are read end to end: {plan}"
    );
    assert!(
        index_rows(&plan, "lyrics_embedding_pending_idx") <= stale,
        "requests inside the result window are walked: {plan}"
    );
    Ok(())
}
