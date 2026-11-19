UPDATE track_video_dispatch
SET payload = jsonb_set(
        jsonb_set(
            payload,
            '{vlm_sent}',
            to_jsonb(COALESCE((payload ->> 'vlm_sent')::int, 0) + 1)
        ),
        '{haiku}',
        COALESCE($3::jsonb, 'null'::jsonb)
    ),
    updated_at = now()
WHERE sc_track_id = $1
  AND attempts = $2
  AND stage = 'vlm_dispatched'
  AND payload IS NOT NULL
RETURNING sc_track_id AS "sc_track_id!",
          attempts AS "attempt!",
          payload AS "payload!",
          (SELECT track.duration_ms FROM tracks AS track WHERE track.sc_track_id = track_video_dispatch.sc_track_id) AS duration_ms
