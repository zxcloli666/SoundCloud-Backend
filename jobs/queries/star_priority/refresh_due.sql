WITH watch AS (
    INSERT INTO star_due_watch (singleton, watched_at)
    VALUES (true, now())
    ON CONFLICT (singleton) DO UPDATE
    SET watched_at = EXCLUDED.watched_at
), marked AS (
    DELETE FROM star_due_marks
    RETURNING sc_track_id
), targets AS MATERIALIZED (
    SELECT star.sc_track_id,
           star.audio_due,
           star.lookup_due,
           star.embed_due,
           star.align_due,
           star.enrich_due,
           star.upgrade_from,
           star.upgrade_tried_at,
           coalesce(star.recheck_at <= now(), false)
               OR star.sc_track_id IN (SELECT marked.sc_track_id FROM marked) AS expected
    FROM star_track_tiers AS star
    WHERE $4::boolean
       OR star.recheck_at <= now()
       OR star.sc_track_id IN (SELECT marked.sc_track_id FROM marked)
), due AS MATERIALIZED (
    SELECT star.sc_track_id,
           CASE
               WHEN stored.index_state = 'pending'
                AND stored.storage_state = 'ok'
                AND stored.s3_verified_at IS NOT NULL
                AND NOT stored.needs_duration_resolve
                AND NOT stored.pipeline_held
                   THEN NOT EXISTS (
                       SELECT 1
                       FROM audio_index_wire_state AS wire
                       WHERE wire.sc_track_id = star.sc_track_id
                         AND (
                             (
                                 wire.upload_generation = (
                                     SELECT storage.uploaded_generation
                                     FROM storage_event_state AS storage
                                     WHERE storage.sc_track_id = star.sc_track_id
                                 )
                                 AND wire.status IN ('terminal', 'reopenable')
                             )
                             OR (
                                 wire.status = 'pending'
                                 AND wire.dispatched_at > now() - $1::bigint * interval '1 second'
                             )
                         )
                   )
               ELSE false
           END AS audio_due,
           coalesce(lookup.next_run_at <= now() AND lookup.claim_job_id IS NULL, false) AS lookup_due,
           CASE
               WHEN lyrics.embedded_at IS NULL
                AND lyrics.embedding_state IS NULL
                AND length(coalesce(lyrics.plain_text, lyrics.synced_lrc, '')) > 30
                   THEN NOT EXISTS (
                       SELECT 1
                       FROM lyrics_embedding_wire_state AS wire
                       WHERE wire.sc_track_id = star.sc_track_id
                         AND wire.status = 'pending'
                   )
               ELSE false
           END AS embed_due,
           CASE
               WHEN stored.storage_state = 'ok'
                AND NOT stored.needs_duration_resolve
                AND (stored.transcribe_state IS NULL OR stored.transcribe_state = 'quarantined')
                AND lyrics.plain_text IS NOT NULL
                   THEN EXISTS (
                            SELECT 1
                            FROM storage_event_state AS event
                            LEFT JOIN transcription_wire_state AS wire
                              ON wire.sc_track_id = event.sc_track_id
                            WHERE event.sc_track_id = star.sc_track_id
                              AND event.uploaded_generation > 0
                              AND (
                                  (
                                      event.transcription_generation IS NULL
                                      AND stored.transcribe_state IS NULL
                                      AND wire.sc_track_id IS NULL
                                  )
                                  OR (
                                      stored.transcribe_state = 'quarantined'
                                      AND wire.status = 'quarantined'
                                      AND wire.quarantine_reason IN ('new_upload_during_pending', 'reopen_superseded')
                                      AND wire.upload_generation < event.uploaded_generation
                                  )
                              )
                        )
                    AND NULLIF(btrim(lyrics.plain_text), '') IS NOT NULL
                    AND NULLIF(btrim(lyrics.synced_lrc), '') IS NULL
               ELSE false
           END AS align_due,
           coalesce(
               stored.enrich_state IN ('pending', 'failed')
               AND stored.enrich_next_run_at <= now()
               AND stored.enrich_attempts < $3,
               false
           ) AS enrich_due,
           CASE
               WHEN stored.hq_upgrade_pending
                AND stored.storage_quality IN ('sq', 'hq')
                AND (
                    stored.hq_upgrade_last_at IS NULL
                    OR stored.hq_upgrade_last_at < now() - $2::bigint * interval '1 second'
                )
                   THEN stored.storage_quality
           END AS upgrade_from,
           stored.hq_upgrade_last_at AS upgrade_tried_at,
           least(
               CASE
                   WHEN stored.index_state = 'pending'
                    AND stored.storage_state = 'ok'
                    AND stored.s3_verified_at IS NOT NULL
                    AND NOT stored.needs_duration_resolve
                    AND NOT stored.pipeline_held
                       THEN (
                           SELECT wire.dispatched_at + $1::bigint * interval '1 second'
                           FROM audio_index_wire_state AS wire
                           WHERE wire.sc_track_id = star.sc_track_id
                             AND wire.status = 'pending'
                             AND wire.dispatched_at > now() - $1::bigint * interval '1 second'
                       )
               END,
               CASE
                   WHEN lookup.claim_job_id IS NULL AND lookup.next_run_at > now()
                       THEN lookup.next_run_at
               END,
               CASE
                   WHEN stored.enrich_state IN ('pending', 'failed')
                    AND stored.enrich_attempts < $3
                    AND stored.enrich_next_run_at > now()
                       THEN stored.enrich_next_run_at
               END,
               CASE
                   WHEN stored.hq_upgrade_pending
                    AND stored.storage_quality IN ('sq', 'hq')
                    AND stored.hq_upgrade_last_at >= now() - $2::bigint * interval '1 second'
                       THEN stored.hq_upgrade_last_at + $2::bigint * interval '1 second'
               END
           ) AS recheck_at
    FROM targets AS star
    LEFT JOIN LATERAL (
        SELECT track.enrich_state,
               track.enrich_next_run_at,
               track.enrich_attempts,
               track.hq_upgrade_pending,
               track.storage_quality,
               track.hq_upgrade_last_at,
               track.index_state,
               track.storage_state,
               track.s3_verified_at,
               track.needs_duration_resolve,
               track.pipeline_held,
               track.transcribe_state
        FROM tracks AS track
        WHERE track.sc_track_id = star.sc_track_id
        LIMIT 1
    ) AS stored ON true
    LEFT JOIN LATERAL (
        SELECT cache.embedded_at,
               cache.embedding_state,
               cache.plain_text,
               cache.synced_lrc
        FROM lyrics_cache AS cache
        WHERE cache.sc_track_id = star.sc_track_id
        LIMIT 1
    ) AS lyrics ON true
    LEFT JOIN LATERAL (
        SELECT state.next_run_at,
               state.claim_job_id
        FROM lyrics_lookup_state AS state
        WHERE state.sc_track_id = star.sc_track_id
        LIMIT 1
    ) AS lookup ON true
), changed AS (
    UPDATE star_track_tiers AS star
    SET audio_due = due.audio_due,
        lookup_due = due.lookup_due,
        embed_due = due.embed_due,
        align_due = due.align_due,
        enrich_due = due.enrich_due,
        upgrade_from = due.upgrade_from,
        upgrade_tried_at = due.upgrade_tried_at,
        recheck_at = due.recheck_at
    FROM due
    WHERE star.sc_track_id = due.sc_track_id
      AND (
          star.audio_due,
          star.lookup_due,
          star.embed_due,
          star.align_due,
          star.enrich_due,
          star.upgrade_from,
          star.upgrade_tried_at,
          star.recheck_at
      ) IS DISTINCT FROM (
          due.audio_due,
          due.lookup_due,
          due.embed_due,
          due.align_due,
          due.enrich_due,
          due.upgrade_from,
          due.upgrade_tried_at,
          due.recheck_at
      )
    RETURNING star.sc_track_id
), compared AS (
    SELECT due.*,
           targets.expected,
           (
               targets.audio_due,
               targets.lookup_due,
               targets.embed_due,
               targets.align_due,
               targets.enrich_due,
               targets.upgrade_from,
               targets.upgrade_tried_at
           ) IS DISTINCT FROM (
               due.audio_due,
               due.lookup_due,
               due.embed_due,
               due.align_due,
               due.enrich_due,
               due.upgrade_from,
               due.upgrade_tried_at
           ) AS flags_changed
    FROM due
    JOIN targets ON targets.sc_track_id = due.sc_track_id
)
SELECT (SELECT count(*) FROM changed) AS "written!",
       count(compared.sc_track_id) AS "rechecked!",
       count(*) FILTER (WHERE compared.flags_changed) AS "changed!",
       count(*) FILTER (WHERE compared.flags_changed AND NOT compared.expected) AS "missed!",
       count(*) FILTER (WHERE coalesce(compared.audio_due, star.audio_due)) AS "audio_due!",
       count(*) FILTER (WHERE coalesce(compared.lookup_due, star.lookup_due)) AS "lookup_due!",
       count(*) FILTER (WHERE coalesce(compared.embed_due, star.embed_due)) AS "embed_due!",
       count(*) FILTER (WHERE coalesce(compared.align_due, star.align_due)) AS "align_due!",
       count(*) FILTER (WHERE coalesce(compared.enrich_due, star.enrich_due)) AS "enrich_due!",
       count(*) FILTER (
           WHERE CASE
                     WHEN compared.sc_track_id IS NULL THEN star.upgrade_from IS NOT NULL
                     ELSE compared.upgrade_from IS NOT NULL
                 END
       ) AS "upgrade_due!"
FROM star_track_tiers AS star
LEFT JOIN compared ON compared.sc_track_id = star.sc_track_id
