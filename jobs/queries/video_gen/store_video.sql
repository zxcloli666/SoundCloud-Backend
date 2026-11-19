INSERT INTO track_videos (sc_track_id, kind, source, source_ref, s3_key, offset_ms, loop, duration_ms, verify)
VALUES ($1, 'generated', 'generated', NULL, $2, $3, true, $4, $5)
ON CONFLICT (sc_track_id, kind) DO UPDATE
SET source = EXCLUDED.source,
    source_ref = EXCLUDED.source_ref,
    s3_key = EXCLUDED.s3_key,
    offset_ms = EXCLUDED.offset_ms,
    loop = EXCLUDED.loop,
    duration_ms = EXCLUDED.duration_ms,
    verify = EXCLUDED.verify,
    created_at = now()
