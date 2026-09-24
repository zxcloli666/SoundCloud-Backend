WITH doomed AS (SELECT track.id
                FROM tracks AS track
                WHERE track.uploader_sc_user_id = $2
                  AND track.primary_artist_id = $1
                  AND track.enrich_source = 'sc_verified'
                  AND track.enrich_locked_at IS NULL
                ORDER BY track.id
                LIMIT $3),
     removed AS (
         DELETE FROM track_artists AS credit
             USING doomed
             WHERE credit.track_id = doomed.id
                 AND credit.artist_id = $1
             RETURNING credit.track_id),
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
             FROM doomed
             WHERE track.id = doomed.id
             RETURNING track.id)
SELECT (SELECT count(*) FROM removed) AS "credits_removed!",
       (SELECT count(*) FROM reset)   AS "tracks_reset!"
