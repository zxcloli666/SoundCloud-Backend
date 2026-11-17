UPDATE video_accounts
SET last_leased_at = now()
WHERE name = (
    SELECT account.name
    FROM video_accounts AS account
    WHERE account.source = $1
      AND account.status = 'ok'
      AND (account.paused_until IS NULL OR account.paused_until <= now())
      AND NOT EXISTS (
          SELECT 1
          FROM track_video_dispatch AS dispatch
          WHERE dispatch.stage = 'dispatched'
            AND dispatch.payload ->> 'account' = account.name
      )
    ORDER BY account.last_leased_at NULLS FIRST, account.name
    LIMIT 1
    FOR UPDATE SKIP LOCKED
)
RETURNING name, secret
