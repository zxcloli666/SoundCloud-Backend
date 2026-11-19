WITH due AS (
    SELECT dispatch.sc_track_id
    FROM track_video_dispatch AS dispatch
    WHERE dispatch.stage = 'found'
      AND EXISTS (
          SELECT 1
          FROM track_videos AS video
          WHERE video.sc_track_id = dispatch.sc_track_id
            AND video.kind IN ('clip', 'loop')
            AND NOT video.verify ? 'vlm'
      )
    ORDER BY dispatch.sc_track_id
    LIMIT $1
    FOR UPDATE SKIP LOCKED
), picked AS (
    SELECT DISTINCT ON (video.sc_track_id)
           video.sc_track_id, video.kind, video.source, video.source_ref, video.s3_key,
           video.offset_ms, video.loop, video.duration_ms, video.verify
    FROM due
    JOIN track_videos AS video ON video.sc_track_id = due.sc_track_id
    WHERE video.kind IN ('clip', 'loop')
      AND NOT video.verify ? 'vlm'
    ORDER BY video.sc_track_id, video.kind
)
UPDATE track_video_dispatch AS dispatch
SET stage = 'pending_vlm',
    next_at = now(),
    payload = jsonb_build_object(
        'kind', picked.kind,
        's3_key', picked.s3_key,
        'file', 'stored',
        'offset_ms', picked.offset_ms,
        'duration_ms', picked.duration_ms,
        'loop', picked.loop,
        'source', picked.source,
        'source_ref', picked.source_ref,
        'scores', picked.verify,
        'found_next_at', dispatch.next_at
    ),
    updated_at = now()
FROM picked
WHERE dispatch.sc_track_id = picked.sc_track_id
RETURNING dispatch.sc_track_id AS "sc_track_id!"
