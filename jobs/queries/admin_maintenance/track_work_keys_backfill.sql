WITH batch AS MATERIALIZED (
    SELECT track.id,
           track.primary_artist_id,
           track.superseded_by IS NOT NULL AS superseded,
           track.work_key,
           track.recording_key,
           track.work_normalizer_version
    FROM tracks AS track
    WHERE track.id > $1
    ORDER BY track.id
    LIMIT $2
), written AS (
    INSERT INTO track_work_keys AS stored (
        track_id, primary_artist_id, superseded, work_key, recording_key, work_normalizer_version
    )
    SELECT batch.id,
           batch.primary_artist_id,
           batch.superseded,
           batch.work_key,
           batch.recording_key,
           batch.work_normalizer_version
    FROM batch
    ORDER BY batch.id
    ON CONFLICT (track_id) DO UPDATE
    SET work_key = EXCLUDED.work_key,
        recording_key = EXCLUDED.recording_key,
        work_normalizer_version = EXCLUDED.work_normalizer_version
    WHERE stored.work_normalizer_version IS NULL
      AND EXCLUDED.work_normalizer_version IS NOT NULL
    RETURNING stored.track_id
)
SELECT (SELECT batch.id FROM batch ORDER BY batch.id DESC LIMIT 1) AS "last?",
       (SELECT count(*) FROM batch) AS "scanned!",
       (SELECT count(*) FROM written) AS "inserted!"
