WITH closed AS (
    UPDATE track_video_dispatch
    SET stage = 'found',
        next_at = COALESCE((payload ->> 'found_next_at')::timestamptz, now()),
        last_error = NULL,
        payload = NULL,
        updated_at = now()
    WHERE sc_track_id = $1
      AND attempts = $2
      AND stage = 'vlm_dispatched'
    RETURNING sc_track_id
), marked AS (
    UPDATE track_videos AS video
    SET verify = video.verify || jsonb_build_object('vlm', $4::jsonb)
    FROM closed
    WHERE video.sc_track_id = closed.sc_track_id
      AND video.s3_key = $3
)
SELECT sc_track_id AS "sc_track_id!"
FROM closed
