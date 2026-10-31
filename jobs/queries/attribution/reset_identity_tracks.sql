WITH doomed AS (SELECT track.id
                FROM tracks AS track
                WHERE track.uploader_sc_user_id = $2
                  AND track.primary_artist_id = $1
                  AND track.enrich_source = 'sc_verified'
                  AND track.enrich_locked_at IS NULL
                ORDER BY track.id
                LIMIT $3),
     credits AS MATERIALIZED (SELECT credit.track_id, credit.artist_id, credit.role
                              FROM track_artists AS credit
                              WHERE credit.track_id IN (SELECT id FROM doomed)
                                AND credit.artist_id = $1
                              ORDER BY credit.track_id, credit.artist_id, credit.role
                              FOR UPDATE OF credit),
     removed AS (
         DELETE FROM track_artists AS credit
             USING credits
             WHERE credit.track_id = credits.track_id
                 AND credit.artist_id = credits.artist_id
                 AND credit.role = credits.role
             RETURNING credit.track_id),
     locked AS MATERIALIZED (SELECT track.id
                             FROM tracks AS track
                             WHERE track.id IN (SELECT id FROM doomed)
                             ORDER BY track.sc_track_id
                             FOR NO KEY UPDATE OF track),
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
             FROM locked
             WHERE track.id = locked.id
             RETURNING track.id)
SELECT (SELECT count(*) FROM removed) AS "credits_removed!",
       (SELECT count(*) FROM reset)   AS "tracks_reset!"
