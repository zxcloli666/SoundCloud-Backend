WITH doomed AS (SELECT credit.track_id,
                       credit.artist_id
                FROM track_artists AS credit
                         JOIN tracks AS track ON track.id = credit.track_id
                WHERE credit.source = 'walker'
                  AND track.enrich_locked_at IS NULL
                ORDER BY credit.track_id
                LIMIT $1),
     credits AS MATERIALIZED (SELECT credit.track_id, credit.artist_id, credit.role
                              FROM track_artists AS credit
                              WHERE (credit.track_id, credit.artist_id) IN (SELECT track_id, artist_id FROM doomed)
                                AND credit.source = 'walker'
                              ORDER BY credit.track_id, credit.artist_id, credit.role
                              FOR UPDATE OF credit),
     removed AS (
         DELETE FROM track_artists AS credit
             USING credits
             WHERE credit.track_id = credits.track_id
                 AND credit.artist_id = credits.artist_id
                 AND credit.role = credits.role
             RETURNING credit.track_id, credit.artist_id),
     locked AS MATERIALIZED (SELECT track.id
                             FROM tracks AS track
                             WHERE EXISTS (SELECT 1
                                           FROM removed
                                           WHERE removed.track_id = track.id
                                             AND removed.artist_id = track.primary_artist_id)
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
