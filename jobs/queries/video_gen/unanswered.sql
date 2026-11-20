UPDATE track_video_gen
SET sent_at = NULL,
    faults = faults + 1,
    next_at = now(),
    updated_at = now()
WHERE sent_at < now() - $1::float8 * interval '1 second'
  AND stage <> ALL ($2::text[])
