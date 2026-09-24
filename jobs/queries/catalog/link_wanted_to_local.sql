WITH due AS (SELECT id, primary_artist_id, work_key, recording_key, duration_ms
             FROM wanted_tracks
             WHERE status = 'wanted'
               AND track_id IS NULL
               AND work_key IS NOT NULL
               AND primary_artist_id IS NOT NULL
               AND (work_reconciled_at IS NULL OR work_reconciled_at < now() - interval '7 days')
             ORDER BY work_reconciled_at NULLS FIRST, id
             LIMIT $1),
     matched AS (SELECT DISTINCT ON (due.id) due.id                                       AS wanted_track_id,
                                             track.id                                     AS track_id,
                                             CASE
                                                 WHEN due.recording_key IS NOT DISTINCT FROM track.recording_key
                                                     THEN 'recording'
                                                 WHEN due.work_key = track.work_key THEN 'work'
                                                 ELSE 'alias' END                         AS match_reason,
                                             track.work_key                               AS matched_key
                 FROM due
                          JOIN tracks AS track
                               ON track.primary_artist_id = due.primary_artist_id
                                   AND track.work_key IS NOT NULL
                                   AND (track.work_key = due.work_key
                                       OR EXISTS (SELECT 1
                                                  FROM track_work_aliases AS alias
                                                  WHERE alias.track_id = track.id
                                                    AND alias.alias_key = due.work_key)
                                       OR EXISTS (SELECT 1
                                                  FROM wanted_track_work_aliases AS alias
                                                  WHERE alias.wanted_track_id = due.id
                                                    AND alias.alias_key = track.work_key))
                                   AND (due.duration_ms IS NULL
                                       OR track.duration_ms IS NULL
                                       OR abs(track.duration_ms - due.duration_ms) <= 7000)
                 ORDER BY due.id,
                          (due.recording_key IS NOT DISTINCT FROM track.recording_key) DESC,
                          (due.work_key = track.work_key) DESC,
                          track.created_at),
     linked AS (
         UPDATE wanted_tracks
             SET track_id = matched.track_id,
                 status = 'linked',
                 work_reconciled_at = now(),
                 updated_at = now()
             FROM matched
             WHERE wanted_tracks.id = matched.wanted_track_id
                 AND wanted_tracks.track_id IS NULL
             RETURNING wanted_tracks.id),
     journal AS (
         INSERT INTO catalog_work_links (wanted_track_id, track_id, match_reason, matched_key,
                                         normalizer_version)
             SELECT matched.wanted_track_id,
                    matched.track_id,
                    matched.match_reason,
                    matched.matched_key,
                    $2
             FROM matched
                      JOIN linked ON linked.id = matched.wanted_track_id
             ON CONFLICT (wanted_track_id, track_id) DO NOTHING),
     examined AS (
         UPDATE wanted_tracks
             SET work_reconciled_at = now()
             FROM due
             WHERE wanted_tracks.id = due.id
                 AND NOT EXISTS (SELECT 1 FROM matched WHERE matched.wanted_track_id = due.id)
             RETURNING wanted_tracks.id)
SELECT (SELECT count(*) FROM linked)   AS "linked!",
       (SELECT count(*) FROM examined) AS "unmatched!"
