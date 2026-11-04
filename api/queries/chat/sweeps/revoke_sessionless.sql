UPDATE chat.devices AS device
SET status = 'revoked',
    revoked_at = now()
WHERE device.id IN (
    SELECT candidate.id
    FROM chat.devices AS candidate
    WHERE candidate.status = 'active'
      AND candidate.last_seen_at < now() - interval '1 hour'
      AND candidate.user_id NOT LIKE 'lt:%'
      AND NOT EXISTS (
          SELECT 1
          FROM public.sessions AS session
          WHERE session.id = candidate.session_id
      )
    ORDER BY candidate.last_seen_at
    LIMIT $1
    FOR UPDATE SKIP LOCKED
)
  AND device.status = 'active'
RETURNING device.id, device.user_id
