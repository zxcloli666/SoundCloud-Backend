UPDATE video_accounts
SET last_leased_at = now()
WHERE name = (
    SELECT account.name
    FROM video_accounts AS account
    WHERE account.source = $1
      AND account.status = 'ok'
      AND (account.paused_until IS NULL OR account.paused_until <= now())
    ORDER BY account.last_leased_at NULLS FIRST, account.name
    LIMIT 1
    FOR UPDATE SKIP LOCKED
)
RETURNING name, secret
