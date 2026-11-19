SELECT reason, paused_until <= now() AS "due!", alerted_at IS NOT NULL AS "alerted!"
FROM llm_pause
WHERE ladder = $1
