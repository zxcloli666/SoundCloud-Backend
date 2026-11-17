UPDATE video_accounts
SET alerted_at = NULL
WHERE name = $1 AND status = 'dead'
