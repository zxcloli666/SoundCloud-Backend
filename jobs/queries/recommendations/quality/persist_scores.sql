UPDATE tracks AS track
SET quality_score = score.value,
    quality_model_version = $3::bigint
FROM UNNEST($1::text[], $2::real[]) AS score(sc_track_id, value)
WHERE track.sc_track_id = score.sc_track_id
