WITH old AS (
    SELECT s3_key
    FROM track_videos
    WHERE sc_track_id = $1
      AND kind = $2
)
INSERT INTO track_videos (sc_track_id, kind, source, source_ref, s3_key, offset_ms, loop, duration_ms, verify)
VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
ON CONFLICT (sc_track_id, kind) DO UPDATE
SET source = EXCLUDED.source,
    source_ref = EXCLUDED.source_ref,
    s3_key = EXCLUDED.s3_key,
    offset_ms = EXCLUDED.offset_ms,
    loop = EXCLUDED.loop,
    duration_ms = EXCLUDED.duration_ms,
    verify = EXCLUDED.verify
RETURNING (SELECT s3_key FROM old) AS replaced
