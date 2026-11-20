WITH due AS (
    SELECT sc_track_id
    FROM track_video_dispatch
    WHERE stage = 'pending_vlm'
      AND payload IS NOT NULL
      AND next_at <= now()
    ORDER BY updated_at
    LIMIT $1
    FOR UPDATE SKIP LOCKED
)
UPDATE track_video_dispatch AS dispatch
SET stage = 'vlm_dispatched',
    payload = jsonb_set(
        dispatch.payload - 'vlm_gpu',
        '{vlm_sent}',
        to_jsonb(COALESCE((dispatch.payload ->> 'vlm_sent')::int, 0) + 1)
    ),
    updated_at = now()
FROM due
WHERE dispatch.sc_track_id = due.sc_track_id
RETURNING dispatch.sc_track_id AS "sc_track_id!",
          dispatch.attempts AS "attempt!",
          dispatch.payload AS "payload!",
          (SELECT track.duration_ms FROM tracks AS track WHERE track.sc_track_id = dispatch.sc_track_id) AS duration_ms
