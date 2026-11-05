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
