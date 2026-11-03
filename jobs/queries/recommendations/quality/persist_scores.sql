WITH score AS (SELECT score.sc_track_id, score.value
               FROM UNNEST($1::text[], $2::real[]) AS score(sc_track_id, value)),
     locked AS MATERIALIZED (SELECT track.id, score.value
                             FROM tracks AS track
                                      JOIN score ON score.sc_track_id = track.sc_track_id
                             WHERE (track.quality_score, track.quality_model_version)
                                       IS DISTINCT FROM (score.value, $3::bigint)
                             ORDER BY track.sc_track_id
                             FOR NO KEY UPDATE OF track)
UPDATE tracks AS track
SET quality_score = locked.value,
    quality_model_version = $3::bigint
FROM locked
WHERE track.id = locked.id
