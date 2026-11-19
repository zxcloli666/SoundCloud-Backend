UPDATE track_video_dispatch
SET stage = 'pending_vlm',
    next_at = now(),
    last_error = 'static_video',
    payload = (payload -> 'next' -> 0)
        || jsonb_build_object(
            'file', 'raw',
            'next', (payload -> 'next') - 0,
            'vlm_sent', COALESCE(payload -> 'vlm_sent', '0'::jsonb)
        ),
    updated_at = now()
WHERE sc_track_id = $1
  AND attempts = $2
  AND stage = 'vlm_dispatched'
  AND jsonb_typeof(payload -> 'next') = 'array'
  AND jsonb_array_length(payload -> 'next') > 0
RETURNING sc_track_id AS "sc_track_id!"
