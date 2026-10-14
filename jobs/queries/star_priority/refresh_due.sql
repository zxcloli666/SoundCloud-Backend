WITH due AS MATERIALIZED (
    SELECT star.sc_track_id,
           audio.sc_track_id IS NOT NULL AS audio_due,
           lookup.sc_track_id IS NOT NULL AS lookup_due,
           embed.sc_track_id IS NOT NULL AS embed_due,
           align.sc_track_id IS NOT NULL AS align_due,
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
           stored.hq_upgrade_last_at AS upgrade_tried_at
    FROM star_track_tiers AS star
    LEFT JOIN LATERAL (
        SELECT track.enrich_state,
               track.enrich_next_run_at,
               track.enrich_attempts,
               track.hq_upgrade_pending,
               track.storage_quality,
               track.hq_upgrade_last_at
        FROM tracks AS track
        WHERE track.sc_track_id = star.sc_track_id
        LIMIT 1
    ) AS stored ON true
    LEFT JOIN LATERAL (
        SELECT track.sc_track_id
        FROM tracks AS track
        LEFT JOIN storage_event_state AS storage
          ON storage.sc_track_id = track.sc_track_id
        WHERE track.sc_track_id = star.sc_track_id
          AND track.index_state = 'pending'
          AND track.storage_state = 'ok'
          AND track.s3_verified_at IS NOT NULL
          AND NOT track.needs_duration_resolve
          AND NOT track.pipeline_held
          AND NOT EXISTS (
              SELECT 1
              FROM audio_index_wire_state AS wire
              WHERE wire.sc_track_id = track.sc_track_id
                AND (
                    (
                        wire.upload_generation = storage.uploaded_generation
                        AND wire.status IN ('terminal', 'reopenable')
                    )
                    OR (
                        wire.status = 'pending'
                        AND wire.dispatched_at > now() - $1::bigint * interval '1 second'
                    )
                )
          )
        LIMIT 1
    ) AS audio ON true
    LEFT JOIN LATERAL (
        SELECT state.sc_track_id
        FROM lyrics_lookup_state AS state
        WHERE state.sc_track_id = star.sc_track_id
          AND state.next_run_at <= now()
          AND state.claim_job_id IS NULL
        LIMIT 1
    ) AS lookup ON true
    LEFT JOIN LATERAL (
        SELECT lyrics.sc_track_id
        FROM lyrics_cache AS lyrics
        WHERE lyrics.sc_track_id = star.sc_track_id
          AND lyrics.embedded_at IS NULL
          AND lyrics.embedding_state IS NULL
          AND length(coalesce(lyrics.plain_text, lyrics.synced_lrc, '')) > 30
          AND NOT EXISTS (
              SELECT 1
              FROM lyrics_embedding_wire_state AS wire
              WHERE wire.sc_track_id = lyrics.sc_track_id
                AND wire.status = 'pending'
          )
        LIMIT 1
    ) AS embed ON true
    LEFT JOIN LATERAL (
        SELECT event.sc_track_id
        FROM storage_event_state AS event
        JOIN tracks AS track
          ON track.sc_track_id = event.sc_track_id
        JOIN lyrics_cache AS lyrics
          ON lyrics.sc_track_id = event.sc_track_id
        LEFT JOIN transcription_wire_state AS wire
          ON wire.sc_track_id = event.sc_track_id
        WHERE event.sc_track_id = star.sc_track_id
          AND NULLIF(btrim(lyrics.plain_text), '') IS NOT NULL
          AND NULLIF(btrim(lyrics.synced_lrc), '') IS NULL
          AND event.uploaded_generation > 0
          AND track.storage_state = 'ok'
          AND NOT track.needs_duration_resolve
          AND (
              (
                  event.transcription_generation IS NULL
                  AND track.transcribe_state IS NULL
                  AND wire.sc_track_id IS NULL
              )
              OR (
                  track.transcribe_state = 'quarantined'
                  AND wire.status = 'quarantined'
                  AND wire.quarantine_reason IN ('new_upload_during_pending', 'reopen_superseded')
                  AND wire.upload_generation < event.uploaded_generation
              )
          )
        LIMIT 1
    ) AS align ON true
), changed AS (
    UPDATE star_track_tiers AS star
    SET audio_due = due.audio_due,
        lookup_due = due.lookup_due,
        embed_due = due.embed_due,
        align_due = due.align_due,
        enrich_due = due.enrich_due,
        upgrade_from = due.upgrade_from,
        upgrade_tried_at = due.upgrade_tried_at
    FROM due
    WHERE star.sc_track_id = due.sc_track_id
      AND (
          star.audio_due,
          star.lookup_due,
          star.embed_due,
          star.align_due,
          star.enrich_due,
          star.upgrade_from,
          star.upgrade_tried_at
      ) IS DISTINCT FROM (
          due.audio_due,
          due.lookup_due,
          due.embed_due,
          due.align_due,
          due.enrich_due,
          due.upgrade_from,
          due.upgrade_tried_at
      )
    RETURNING star.sc_track_id
)
SELECT (SELECT count(*) FROM changed) AS "changed!",
       count(*) FILTER (WHERE due.audio_due) AS "audio_due!",
       count(*) FILTER (WHERE due.lookup_due) AS "lookup_due!",
       count(*) FILTER (WHERE due.embed_due) AS "embed_due!",
       count(*) FILTER (WHERE due.align_due) AS "align_due!",
       count(*) FILTER (WHERE due.enrich_due) AS "enrich_due!",
       count(*) FILTER (WHERE due.upgrade_from IS NOT NULL) AS "upgrade_due!"
FROM due
