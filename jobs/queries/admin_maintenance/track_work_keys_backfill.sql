WITH batch AS MATERIALIZED (
    SELECT track.id,
           track.primary_artist_id,
           track.superseded_by IS NOT NULL AS superseded
    FROM tracks AS track
    WHERE track.id > $1
    ORDER BY track.id
    LIMIT $2
), written AS (
    INSERT INTO track_work_keys (track_id, primary_artist_id, superseded)
    SELECT batch.id, batch.primary_artist_id, batch.superseded
    FROM batch
    ORDER BY batch.id
    ON CONFLICT (track_id) DO NOTHING
    RETURNING track_work_keys.track_id
)
SELECT (SELECT batch.id FROM batch ORDER BY batch.id DESC LIMIT 1) AS "last?",
       (SELECT count(*) FROM batch) AS "scanned!",
       (SELECT count(*) FROM written) AS "inserted!"
