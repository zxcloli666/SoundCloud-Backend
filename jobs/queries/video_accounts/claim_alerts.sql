UPDATE video_accounts
SET alerted_at = now()
WHERE status = 'dead' AND alerted_at IS NULL
RETURNING name, source, last_error
