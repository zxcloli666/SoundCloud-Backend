UPDATE track_video_gen
SET sent_at = now(),
    updated_at = now()
WHERE sent_at IS NOT NULL
  AND stage = ANY ($1::text[])
