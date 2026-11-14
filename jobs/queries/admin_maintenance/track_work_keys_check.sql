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
), bounds AS MATERIALIZED (
    SELECT last.id AS last,
           CASE
               WHEN (SELECT count(*) FROM batch) < $2 THEN 'ffffffff-ffff-ffff-ffff-ffffffffffff'::uuid
               ELSE last.id
           END AS upper,
           (SELECT count(*) FROM batch) AS scanned
    FROM (SELECT (SELECT batch.id FROM batch ORDER BY batch.id DESC LIMIT 1) AS id) AS last
), stored AS MATERIALIZED (
    SELECT keys.track_id,
           keys.primary_artist_id,
           keys.superseded,
           keys.work_key,
           keys.recording_key,
           keys.work_normalizer_version
    FROM bounds
    JOIN track_work_keys AS keys ON keys.track_id > $1 AND keys.track_id <= bounds.upper
)
SELECT bounds.last AS "last?",
       bounds.scanned AS "scanned!",
       ARRAY(
           SELECT coalesce(batch.id, stored.track_id)
           FROM batch
           FULL JOIN stored ON stored.track_id = batch.id
           WHERE batch.id IS NULL
              OR stored.track_id IS NULL
              OR (batch.primary_artist_id, batch.superseded, batch.work_key, batch.recording_key,
                  batch.work_normalizer_version)
                 IS DISTINCT FROM
                 (stored.primary_artist_id, stored.superseded, stored.work_key, stored.recording_key,
                  stored.work_normalizer_version)
           ORDER BY 1
       ) AS "mismatched!"
FROM bounds
