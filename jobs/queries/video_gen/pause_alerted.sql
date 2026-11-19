UPDATE video_gen_pause
SET alerted_at = now(),
    updated_at = now()
WHERE id = 1
