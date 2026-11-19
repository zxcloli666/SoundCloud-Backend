SELECT reason, paused_until <= now() AS "due!", alerted_at IS NOT NULL AS "alerted!"
FROM video_gen_pause
WHERE id = 1
