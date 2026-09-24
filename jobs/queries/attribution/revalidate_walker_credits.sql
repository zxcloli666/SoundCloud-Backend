WITH doomed AS (SELECT credit.track_id,
                       credit.artist_id
                FROM track_artists AS credit
                         JOIN tracks AS track ON track.id = credit.track_id
                WHERE credit.source = 'walker'
                  AND track.enrich_locked_at IS NULL
                ORDER BY credit.track_id
                LIMIT $1),
     removed AS (
         DELETE FROM track_artists AS credit
             USING doomed
             WHERE credit.track_id = doomed.track_id
                 AND credit.artist_id = doomed.artist_id
                 AND credit.source = 'walker'
             RETURNING credit.track_id, credit.artist_id),
     reset AS (
         UPDATE tracks AS track
             SET primary_artist_id = NULL,
                 enrich_state = 'pending',
                 enrich_source = NULL,
                 enrich_confidence = NULL,
                 enrich_error = NULL,
                 enrich_attempts = 0,
                 enrich_next_run_at = now() + random() * interval '30 minutes',
                 updated_at = now()
             FROM removed
             WHERE track.id = removed.track_id
                 AND track.primary_artist_id = removed.artist_id
             RETURNING track.id)
SELECT (SELECT count(*) FROM removed) AS "credits_removed!",
       (SELECT count(*) FROM reset)   AS "tracks_reset!"
