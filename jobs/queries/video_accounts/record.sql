UPDATE video_accounts
SET status = COALESCE($2::text, status),
    last_checked_at = CASE WHEN $2::text IS NULL THEN last_checked_at ELSE now() END,
    last_error = $3,
    alerted_at = CASE WHEN $2::text = 'ok' THEN NULL ELSE alerted_at END,
    updated_at = now()
WHERE name = $1
