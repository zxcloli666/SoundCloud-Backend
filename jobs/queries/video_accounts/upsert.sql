INSERT INTO video_accounts (source, name, secret, status, last_checked_at)
VALUES ($1, $2, $3, 'ok', now())
ON CONFLICT (name) DO UPDATE SET
    secret = EXCLUDED.secret,
    status = 'ok',
    last_checked_at = now(),
    last_error = NULL,
    alerted_at = NULL,
    updated_at = now()
WHERE video_accounts.source = EXCLUDED.source
RETURNING name
