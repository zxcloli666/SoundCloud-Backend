WITH batch AS MATERIALIZED (
    SELECT track.id,
           track.sc_synced_at
    FROM tracks AS track
    WHERE track.id > $1
    ORDER BY track.id
    LIMIT $2
), inserted AS (
    INSERT INTO track_sync_stamps (track_id, sc_synced_at)
    SELECT batch.id,
           batch.sc_synced_at
    FROM batch
    ORDER BY batch.id
    ON CONFLICT (track_id) DO NOTHING
    RETURNING track_id
)
SELECT (SELECT batch.id FROM batch ORDER BY batch.id DESC LIMIT 1) AS "last?",
       (SELECT count(*) FROM batch) AS "scanned!",
       (SELECT count(*) FROM inserted) AS "inserted!"
