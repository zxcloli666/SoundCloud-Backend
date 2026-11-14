WITH batch AS MATERIALIZED (
    SELECT track.id,
           track.sc_synced_at
    FROM tracks AS track
    WHERE track.id > $1
    ORDER BY track.id
    LIMIT $2
), bounds AS MATERIALIZED (
    SELECT last.id AS last,
           CASE
               WHEN (SELECT count(*) FROM batch) < $2 THEN 'ffffffff-ffff-ffff-ffff-ffffffffffff'::uuid
               ELSE last.id
           END AS upper,
           (SELECT count(*) FROM batch) AS scanned
    FROM (SELECT (SELECT batch.id FROM batch ORDER BY batch.id DESC LIMIT 1) AS id) AS last
), stored AS MATERIALIZED (
    SELECT stamp.track_id,
           stamp.sc_synced_at
    FROM bounds
    JOIN track_sync_stamps AS stamp ON stamp.track_id > $1 AND stamp.track_id <= bounds.upper
)
SELECT bounds.last AS "last?",
       bounds.scanned AS "scanned!",
       ARRAY(
           SELECT coalesce(batch.id, stored.track_id)
           FROM batch
           FULL JOIN stored ON stored.track_id = batch.id
           WHERE batch.id IS NULL
              OR stored.track_id IS NULL
              OR stored.sc_synced_at < batch.sc_synced_at
           ORDER BY 1
       ) AS "mismatched!"
FROM bounds
