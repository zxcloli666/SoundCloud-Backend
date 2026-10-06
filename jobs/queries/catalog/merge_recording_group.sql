WITH members AS (SELECT track.id,
                        track.duration_ms,
                        row_number() OVER (
                            ORDER BY (track.storage_state = 'ok' AND track.s3_verified_at IS NOT NULL) DESC,
                                (track.index_state = 'ok') DESC,
                                track.quality_score DESC NULLS LAST,
                                track.play_count_sc DESC NULLS LAST,
                                track.created_at,
                                track.id
                            ) AS rank
                 FROM tracks AS track
                 WHERE track.primary_artist_id = $1
                   AND track.recording_key = $2
                   AND track.superseded_by IS NULL),
     winner AS (SELECT id, duration_ms FROM members WHERE rank = 1),
     losers AS (SELECT members.id
                FROM members,
                     winner
                WHERE members.id <> winner.id
                  AND (members.duration_ms IS NULL
                    OR winner.duration_ms IS NULL
                    OR abs(members.duration_ms - winner.duration_ms) <= 5000)),
     grouped AS (
         UPDATE tracks
             SET canonical_track_id = coalesce(
                 (SELECT track.canonical_track_id
                  FROM tracks AS track
                  WHERE track.id IN (SELECT id FROM winner UNION ALL SELECT id FROM losers)
                    AND track.canonical_track_id IS NOT NULL
                  ORDER BY track.canonical_track_id
                  LIMIT 1),
                 $3)
             WHERE id IN (SELECT id FROM winner UNION ALL SELECT id FROM losers)
                 AND canonical_track_id IS NULL
             RETURNING id),
     superseded AS (
         UPDATE tracks
             SET superseded_by = (SELECT id FROM winner),
                 updated_at = now()
             FROM losers
             WHERE tracks.id = losers.id
             RETURNING tracks.id)
SELECT (SELECT count(*) FROM superseded) AS "superseded!",
       (SELECT count(*) FROM grouped)    AS "grouped!"
