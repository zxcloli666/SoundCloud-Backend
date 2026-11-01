SELECT setseed(0.4242);

CREATE FUNCTION pg_temp.planfixture_transcription(key bigint) RETURNS text
    LANGUAGE sql IMMUTABLE
    AS $$
        SELECT CASE
                   WHEN abs(hashint8(key)::bigint) % 1000 < 538 THEN 'rejected'
                   WHEN abs(hashint8(key)::bigint) % 1000 < 900 THEN 'done'
                   WHEN abs(hashint8(key)::bigint) % 1000 < 995 THEN 'quarantined'
                   ELSE 'pending'
               END
    $$;

CREATE TEMP TABLE planfixture_star AS
SELECT k,
       CASE WHEN k % 997 = 0 THEN (9000000000 + k)::text ELSE (45 * k)::text END AS sc_track_id,
       ('00000000-0000-4000-8000-' || lpad(to_hex(45 * k), 12, '0'))::uuid AS track_id,
       k % 997 <> 0 AS stored,
       k % 10 AS a,
       (k / 10) % 10 AS b,
       (k / 100) % 10 AS c,
       (k / 1000) % 10 AS d,
       (k / 7) % 10 AS e,
       (k / 3) % 10 AS f,
       1 + k % 3 AS generation,
       (k / 7) % 10 IN (0, 2, 3) AND k % 8 = 0 AS plain_only,
       CASE
           WHEN k % 997 <> 0 AND k % 10 < 8 AND (k / 7) % 10 IN (0, 2, 3) AND k % 8 = 0 AND k % 97 <> 0
               THEN pg_temp.planfixture_transcription(k)
       END AS transcription
  FROM generate_series(1, 145000) AS k;

CREATE TEMP TABLE planfixture_filler AS
SELECT n,
       n::text AS sc_track_id,
       CASE
           WHEN n <= 1350000 AND n % 100 < 16 AND (n / 17) % 97 <> 0
               THEN pg_temp.planfixture_transcription(n)
       END AS transcription
  FROM generate_series(22, 6600000, 17) AS n
 WHERE n % 45 <> 0;

ALTER TABLE tracks DISABLE TRIGGER ALL;
ALTER TABLE lyrics_cache DISABLE TRIGGER ALL;
ALTER TABLE lyrics_lookup_state DISABLE TRIGGER ALL;
ALTER TABLE storage_event_state DISABLE TRIGGER ALL;
ALTER TABLE audio_index_wire_state DISABLE TRIGGER ALL;
ALTER TABLE transcription_wire_state DISABLE TRIGGER ALL;
ALTER TABLE lyrics_embedding_wire_state DISABLE TRIGGER ALL;
ALTER TABLE star_track_tiers DISABLE TRIGGER ALL;
ALTER TABLE subscriptions DISABLE TRIGGER ALL;
ALTER TABLE user_likes_tracks DISABLE TRIGGER ALL;
ALTER TABLE user_events DISABLE TRIGGER ALL;

UPDATE tracks AS track
   SET storage_state = CASE WHEN star.a < 8 THEN 'ok' WHEN star.a = 8 THEN 'pending' ELSE 'failed' END,
       s3_verified_at = CASE WHEN star.a < 7 THEN now() - (star.k % 500) * interval '1 hour' END,
       index_state = CASE WHEN star.b < 7 THEN 'pending' WHEN star.b < 9 THEN 'indexed' ELSE 'failed' END,
       needs_duration_resolve = star.c = 9 AND star.b < 2,
       pipeline_held = star.c = 8 AND star.b IN (2, 3),
       transcribe_state = star.transcription,
       enrich_state = (ARRAY['pending', 'failed', 'dead', 'done', 'done', 'done', 'done', 'done', 'done', 'done'])[1 + star.d],
       enrich_next_run_at = now() + (star.k % 3 - 1) * interval '1 hour',
       enrich_attempts = star.k % 8,
       hq_upgrade_pending = star.k % 7 = 0,
       storage_quality = (ARRAY['sq', 'hq', 'uq'])[1 + star.k % 3],
       hq_upgrade_last_at = CASE WHEN star.k % 2 = 1 THEN now() - (star.k % 48) * interval '1 hour' END
  FROM planfixture_star AS star
 WHERE star.stored
   AND track.id = star.track_id;

UPDATE tracks AS track
   SET storage_state = 'ok',
       s3_verified_at = now() - (filler.n % 900) * interval '1 hour',
       transcribe_state = filler.transcription
  FROM planfixture_filler AS filler
 WHERE track.id = ('00000000-0000-4000-8000-' || lpad(to_hex(filler.n), 12, '0'))::uuid;

INSERT INTO storage_event_state (
    sc_track_id, stream, stream_sequence, event_published_at, uploaded_generation,
    transcription_generation, updated_at
)
SELECT sc_track_id, 'storage', k, now() - (k % 900) * interval '1 hour', generation,
       CASE
           WHEN transcription = 'quarantined' THEN greatest(generation - 1, 1)
           WHEN transcription IS NOT NULL THEN generation
       END,
       now() - (k % 900) * interval '1 hour'
  FROM planfixture_star
 WHERE stored AND a < 8
UNION ALL
SELECT sc_track_id, 'storage', 1000000 + n, now() - (n % 900) * interval '1 hour', 1,
       CASE WHEN transcription IS NOT NULL THEN 1 END,
       now() - (n % 900) * interval '1 hour'
  FROM planfixture_filler
ON CONFLICT DO NOTHING;

INSERT INTO audio_index_wire_state (
    sc_track_id, status, upload_generation, dispatched_at, completed_at,
    outcome_status, outcome_reason, quarantine_reason
)
SELECT sc_track_id,
       (ARRAY['terminal', 'terminal', 'pending', 'pending', 'reopenable', 'quarantined'])[1 + d],
       CASE WHEN d = 1 THEN greatest(generation - 1, 1) ELSE generation END,
       CASE WHEN d = 2 THEN now() - interval '5 minutes' ELSE now() - interval '2 days' END,
       CASE WHEN d IN (0, 1, 4) THEN now() - interval '1 day' END,
       CASE WHEN d IN (0, 1, 4) THEN 'ok' END,
       CASE WHEN d IN (0, 1, 4) THEN 'indexed' END,
       CASE WHEN d = 5 THEN 'new_upload_during_pending' END
  FROM planfixture_star
 WHERE stored AND d < 6
UNION ALL
SELECT n::text, 'terminal', 1, now() - interval '30 days', now() - interval '30 days', 'ok', 'indexed', NULL
  FROM generate_series(41, 6600000, 41) AS n
 WHERE n % 45 <> 0
ON CONFLICT DO NOTHING;

INSERT INTO transcription_sync_versions (sync_version)
VALUES ('planfixture-v1')
ON CONFLICT DO NOTHING;

INSERT INTO transcription_wire_state (
    sc_track_id, status, upload_generation, dispatched_at, completed_at, quarantine_reason, reason,
    sync_version
)
SELECT wired.sc_track_id,
       wired.transcription,
       wired.generation,
       now() - (wired.key % 40) * interval '1 day' - interval '1 hour',
       CASE
           WHEN wired.transcription IN ('done', 'rejected')
               THEN now() - (wired.key % 40) * interval '1 day'
       END,
       CASE
           WHEN wired.transcription <> 'quarantined' THEN NULL
           WHEN wired.key % 2 = 0 THEN 'new_upload_during_pending'
           WHEN wired.key % 3 = 0 THEN 'reopen_superseded'
           ELSE 'result_timeout'
       END,
       CASE WHEN wired.transcription = 'rejected' THEN 'low_confidence' END,
       CASE WHEN wired.transcription IN ('done', 'rejected') THEN 'planfixture-v1' END
  FROM (
      SELECT sc_track_id, k::bigint AS key, transcription,
             CASE WHEN transcription = 'quarantined' THEN greatest(generation - 1, 1) ELSE generation END AS generation
        FROM planfixture_star
       WHERE transcription IS NOT NULL
      UNION ALL
      SELECT sc_track_id, n, transcription, 1
        FROM planfixture_filler
       WHERE transcription IS NOT NULL
  ) AS wired
ON CONFLICT DO NOTHING;

UPDATE lyrics_cache
   SET synced_lrc = '[00:01.00] ' || plain_text
 WHERE sc_track_id::bigint % 100 >= 16;

DELETE FROM lyrics_cache AS cache
 USING planfixture_star AS star
 WHERE cache.sc_track_id = star.sc_track_id;

INSERT INTO lyrics_cache (
    sc_track_id, track_id, plain_text, synced_lrc, source, embedded_at, embedding_state, created_at
)
SELECT sc_track_id,
       CASE WHEN stored THEN track_id END,
       CASE
           WHEN e IN (0, 1, 2) THEN 'first line of the planfixture lyrics ' || k
           WHEN e = 3 THEN 'la la la'
           WHEN e = 5 THEN '   '
       END,
       CASE
           WHEN e IN (1, 4, 5) OR NOT plain_only
               THEN '[00:01.00] first line of the planfixture lyrics ' || k
       END,
       CASE WHEN e IN (1, 4) THEN 'lrclib' ELSE 'genius' END,
       CASE WHEN e = 1 THEN now() - interval '1 day' END,
       CASE WHEN e = 1 THEN 'done' WHEN e = 2 THEN 'queued' END,
       (now() AT TIME ZONE 'UTC') - (k % 300) * interval '1 day' - interval '1 hour'
  FROM planfixture_star
 WHERE e < 6;

INSERT INTO lyrics_embedding_wire_state (
    sc_track_id, status, lyrics_created_at, request_version, request_text, request_sha256,
    request_message_id, first_publish_attempt_at, publish_retry_until, completed_at
)
SELECT sc_track_id,
       CASE WHEN e = 0 AND k % 3 = 1 THEN 'done' ELSE 'pending' END,
       now() - interval '1 day',
       1,
       'planfixture lyrics ' || k,
       sha256(('planfixture lyrics ' || k)::bytea),
       'planfixture-' || k,
       now() - interval '1 hour',
       now() + interval '1 hour',
       CASE WHEN e = 0 AND k % 3 = 1 THEN now() - interval '30 minutes' END
  FROM planfixture_star
 WHERE (e = 0 AND k % 3 IN (0, 1)) OR (e = 4 AND k % 2 = 0)
UNION ALL
SELECT n::text, 'done', NULL, NULL, NULL, NULL, NULL, NULL, NULL, now() - interval '30 days'
  FROM generate_series(7, 6600000, 10) AS n
 WHERE n % 45 <> 0
ON CONFLICT DO NOTHING;

INSERT INTO lyrics_lookup_state (
    track_id, sc_track_id, priority, input_title, input_artist, input_duration_ms, created_at
)
SELECT track_id, sc_track_id, 0, 'planfixture' || k, 'planfixture artist', 180000, now() - interval '30 days'
  FROM planfixture_star
 WHERE stored
ON CONFLICT DO NOTHING;

UPDATE lyrics_lookup_state AS state
   SET next_run_at = CASE
                         WHEN star.f < 3 THEN now() - interval '1 hour'
                         ELSE now() + (1 + star.k % 30) * interval '1 day'
                     END,
       claim_job_id = CASE WHEN star.f = 2 THEN gen_random_uuid() END,
       claim_job_generation = CASE WHEN star.f = 2 THEN 1 END,
       claim_job_lease_id = CASE WHEN star.f = 2 THEN gen_random_uuid() END,
       claim_state_generation = CASE WHEN star.f = 2 THEN 1 END,
       claim_expires_at = CASE WHEN star.f = 2 THEN now() + interval '10 minutes' END
  FROM planfixture_star AS star
 WHERE state.sc_track_id = star.sc_track_id;

UPDATE lyrics_lookup_backfill_state
   SET completed = true,
       cursor_id = ('00000000-0000-4000-8000-' || lpad(to_hex(6600000), 12, '0'))::uuid
 WHERE singleton;

INSERT INTO subscriptions (user_urn, exp_date)
SELECT (95000 + i)::text, extract(epoch FROM now())::bigint + 30 * 86400
  FROM generate_series(1, 200) AS i
ON CONFLICT DO NOTHING;

INSERT INTO user_likes_tracks (user_id, sc_track_id)
SELECT (95001 + k % 200)::text, sc_track_id
  FROM planfixture_star
ON CONFLICT DO NOTHING;

INSERT INTO star_track_tiers (sc_track_id, tier, tiered_at)
SELECT sc_track_id, CASE WHEN k % 5 = 0 THEN 3 ELSE 1 END, now() - (k % 5000) * interval '1 minute'
  FROM planfixture_star
ON CONFLICT DO NOTHING;

CREATE TEMP TABLE planfixture_taste_users AS
SELECT u, least(105000, greatest(1, round(exp(4.13 + 2.0 * random_normal()))))::int AS events
  FROM generate_series(1, coalesce(nullif(current_setting('planfixture.taste_users', true), '')::int, 67000)) AS u;

CREATE TEMP TABLE planfixture_clock AS
SELECT now() AT TIME ZONE 'UTC' AS at;

CREATE FUNCTION pg_temp.planfixture_unit(key bigint, salt bigint) RETURNS float8
    LANGUAGE sql IMMUTABLE
    AS $$ SELECT (hashint8extended(key, salt) & 4503599627370495)::float8 / 4503599627370496 $$;

CREATE TEMP VIEW planfixture_taste_events AS
SELECT drawn.sc_user_id,
       (1 + floor(drawn.pick * 6600000))::int::text AS sc_track_id,
       drawn.event_type,
       CASE drawn.event_type
           WHEN 'full_play' THEN 0.3
           WHEN 'like' THEN 1.0
           WHEN 'playlist_add' THEN 0.9
           WHEN 'dislike' THEN -1.0
           ELSE (ARRAY[-0.8, -0.3, 0.0])[1 + floor(drawn.strength * 3)::int]
       END AS weight,
       clock.at - interval '280 days' * (1 - sqrt(1 - drawn.age)) AS created_at
  FROM (
      SELECT CASE WHEN (taste.u + slot.i) % 3 = 0 THEN 'soundcloud:users:' ELSE '' END || (100000000 + taste.u) AS sc_user_id,
             pg_temp.planfixture_unit(taste.u::bigint * 200000 + slot.i, 1) AS pick,
             CASE
                 WHEN pg_temp.planfixture_unit(taste.u::bigint * 200000 + slot.i, 2) < 0.45 THEN 'full_play'
                 WHEN pg_temp.planfixture_unit(taste.u::bigint * 200000 + slot.i, 2) < 0.80 THEN 'skip'
                 WHEN pg_temp.planfixture_unit(taste.u::bigint * 200000 + slot.i, 2) < 0.88 THEN 'like'
                 WHEN pg_temp.planfixture_unit(taste.u::bigint * 200000 + slot.i, 2) < 0.95 THEN 'playlist_add'
                 ELSE 'dislike'
             END AS event_type,
             pg_temp.planfixture_unit(taste.u::bigint * 200000 + slot.i, 3) AS strength,
             pg_temp.planfixture_unit(taste.u::bigint * 200000 + slot.i, 4) AS age
        FROM planfixture_taste_users AS taste
       CROSS JOIN LATERAL generate_series(1, taste.events) AS slot(i)
  ) AS drawn
 CROSS JOIN planfixture_clock AS clock;

SET work_mem = '64MB';

INSERT INTO user_events (id, sc_user_id, sc_track_id, event_type, weight, created_at)
SELECT ('00000000-0000-4000-8002-' || lpad(to_hex(row_number() OVER (ORDER BY event.created_at)), 12, '0'))::uuid,
       event.sc_user_id, event.sc_track_id, event.event_type, event.weight, event.created_at
  FROM planfixture_taste_events AS event
 WHERE event.created_at <= (SELECT at FROM planfixture_clock) - interval '7 days'
 ORDER BY event.created_at;

DROP TABLE IF EXISTS planfixture_late_events;
CREATE TABLE planfixture_late_events (LIKE user_events);

INSERT INTO planfixture_late_events (id, sc_user_id, sc_track_id, event_type, weight, created_at)
SELECT ('00000000-0000-4000-8005-' || lpad(to_hex(row_number() OVER (ORDER BY event.created_at)), 12, '0'))::uuid,
       event.sc_user_id, event.sc_track_id, event.event_type, event.weight, event.created_at
  FROM planfixture_taste_events AS event
 WHERE event.created_at > (SELECT at FROM planfixture_clock) - interval '7 days'
 ORDER BY event.created_at;

RESET work_mem;

ALTER TABLE tracks ENABLE TRIGGER ALL;
ALTER TABLE lyrics_cache ENABLE TRIGGER ALL;
ALTER TABLE lyrics_lookup_state ENABLE TRIGGER ALL;
ALTER TABLE storage_event_state ENABLE TRIGGER ALL;
ALTER TABLE audio_index_wire_state ENABLE TRIGGER ALL;
ALTER TABLE transcription_wire_state ENABLE TRIGGER ALL;
ALTER TABLE lyrics_embedding_wire_state ENABLE TRIGGER ALL;
ALTER TABLE star_track_tiers ENABLE TRIGGER ALL;
ALTER TABLE subscriptions ENABLE TRIGGER ALL;
ALTER TABLE user_likes_tracks ENABLE TRIGGER ALL;
ALTER TABLE user_events ENABLE TRIGGER ALL;

ALTER TABLE tracks ENABLE ALWAYS TRIGGER star_due_mark_update;
ALTER TABLE tracks ENABLE ALWAYS TRIGGER star_due_mark_row;
ALTER TABLE lyrics_cache ENABLE ALWAYS TRIGGER star_due_mark_update;
ALTER TABLE lyrics_cache ENABLE ALWAYS TRIGGER star_due_mark_row;
ALTER TABLE lyrics_lookup_state ENABLE ALWAYS TRIGGER star_due_mark_update;
ALTER TABLE lyrics_lookup_state ENABLE ALWAYS TRIGGER star_due_mark_row;
ALTER TABLE storage_event_state ENABLE ALWAYS TRIGGER star_due_mark_update;
ALTER TABLE storage_event_state ENABLE ALWAYS TRIGGER star_due_mark_row;
ALTER TABLE audio_index_wire_state ENABLE ALWAYS TRIGGER star_due_mark_update;
ALTER TABLE audio_index_wire_state ENABLE ALWAYS TRIGGER star_due_mark_row;
ALTER TABLE transcription_wire_state ENABLE ALWAYS TRIGGER star_due_mark_update;
ALTER TABLE transcription_wire_state ENABLE ALWAYS TRIGGER star_due_mark_row;
ALTER TABLE lyrics_embedding_wire_state ENABLE ALWAYS TRIGGER star_due_mark_update;
ALTER TABLE lyrics_embedding_wire_state ENABLE ALWAYS TRIGGER star_due_mark_row;

SELECT 'star_track_tiers' AS seeded, count(*) FROM star_track_tiers
UNION ALL SELECT 'storage_event_state', count(*) FROM storage_event_state
UNION ALL SELECT 'audio_index_wire_state', count(*) FROM audio_index_wire_state
UNION ALL SELECT 'storage_event_reap', count(*) FROM storage_event_state
           WHERE uploaded_generation > 0 AND transcription_generation IS NULL
UNION ALL SELECT 'transcription_wire_state', count(*) FROM transcription_wire_state
UNION ALL SELECT 'transcription_wire_' || status, count(*) FROM transcription_wire_state GROUP BY status
UNION ALL SELECT 'lyrics_plain_only', count(*) FROM lyrics_cache
           WHERE NULLIF(btrim(plain_text), '') IS NOT NULL AND NULLIF(btrim(synced_lrc), '') IS NULL
UNION ALL SELECT 'lyrics_embedding_wire_state', count(*) FROM lyrics_embedding_wire_state
UNION ALL SELECT 'user_events', count(*) FROM user_events
UNION ALL SELECT 'planfixture_late_events', count(*) FROM planfixture_late_events;
