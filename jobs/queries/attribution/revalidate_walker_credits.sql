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
     locked AS (SELECT array_agg(ordered.id) AS ids
                FROM (SELECT track.id
                      FROM tracks AS track
                      WHERE EXISTS (SELECT 1
                                    FROM removed
                                    WHERE removed.track_id = track.id
                                      AND removed.artist_id = track.primary_artist_id)
                      ORDER BY track.sc_track_id
                      FOR NO KEY UPDATE OF track) AS ordered),
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
                 AND track.id = ANY ((SELECT ids FROM locked)::uuid[])
             RETURNING track.id)
SELECT (SELECT count(*) FROM removed) AS "credits_removed!",
       (SELECT count(*) FROM reset)   AS "tracks_reset!"
