WITH members AS (SELECT track.id,
                        track.duration_ms,
                        (track.sharing = 'public' AND track.deleted_at IS NULL) AS serving,
                        row_number() OVER (
                            ORDER BY (track.sharing = 'public' AND track.deleted_at IS NULL) DESC,
                                (track.storage_state = 'ok' AND track.s3_verified_at IS NOT NULL) DESC,
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
     winner AS (SELECT id, duration_ms FROM members WHERE rank = 1 AND serving),
     losers AS (SELECT members.id
                FROM members,
                     winner
                WHERE members.id <> winner.id
                  AND members.duration_ms IS NOT NULL
                  AND winner.duration_ms IS NOT NULL
                  AND abs(members.duration_ms - winner.duration_ms) <= 5000),
     grouped AS (SELECT id FROM winner UNION ALL SELECT id FROM losers),
     locked AS MATERIALIZED (SELECT track.id, track.canonical_track_id
                             FROM tracks AS track
                             WHERE track.id IN (SELECT id FROM grouped)
                             ORDER BY track.sc_track_id
                             FOR NO KEY UPDATE OF track),
     canonical AS (SELECT coalesce(
                                  (SELECT locked.canonical_track_id
                                   FROM locked
                                   WHERE locked.canonical_track_id IS NOT NULL
                                   ORDER BY locked.canonical_track_id
                                   LIMIT 1),
                                  $3) AS id),
     merged AS (
         UPDATE tracks AS track
             SET canonical_track_id = coalesce(track.canonical_track_id, (SELECT id FROM canonical)),
                 superseded_by = CASE
                                     WHEN track.id IN (SELECT id FROM losers) THEN (SELECT id FROM winner)
                                     ELSE track.superseded_by
                     END,
                 updated_at = CASE
                                  WHEN track.id IN (SELECT id FROM losers) THEN now()
                                  ELSE track.updated_at
                     END
             FROM locked
             WHERE track.id = locked.id
                 AND (track.canonical_track_id IS NULL OR track.id IN (SELECT id FROM losers))
             RETURNING track.id IN (SELECT id FROM losers) AS superseded)
SELECT (SELECT count(*) FROM merged WHERE superseded)                     AS "superseded!",
       (SELECT count(*) FROM locked WHERE locked.canonical_track_id IS NULL) AS "grouped!"
