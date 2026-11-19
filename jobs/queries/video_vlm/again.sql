WITH tried AS (
    SELECT sc_track_id,
           COALESCE((payload ->> 'vlm_tries')::int, 0) + 1 AS tries
    FROM track_video_dispatch
    WHERE sc_track_id = $1
      AND attempts = $2
      AND stage = 'vlm_dispatched'
    FOR UPDATE
)
UPDATE track_video_dispatch AS dispatch
SET stage = CASE
        WHEN tried.tries < $4 THEN 'pending_vlm'
        WHEN EXISTS (
            SELECT 1
            FROM track_videos AS video
            WHERE video.sc_track_id = dispatch.sc_track_id
              AND video.kind IN ('clip', 'loop')
        ) THEN 'found'
        ELSE 'none'
    END,
    next_at = now() + make_interval(
        secs => CASE WHEN tried.tries >= $4 THEN $6::float8 ELSE $3::float8 * tried.tries END
    ),
    last_error = $5,
    payload = CASE
        WHEN tried.tries >= $4 THEN NULL
        ELSE jsonb_set(dispatch.payload, '{vlm_tries}', to_jsonb(tried.tries))
    END,
    updated_at = now()
FROM tried
WHERE dispatch.sc_track_id = tried.sc_track_id
RETURNING dispatch.stage AS "stage!"
