WITH alive AS MATERIALIZED (
    SELECT public.auth_sessions_alive($2) AS session_id
)
UPDATE chat.devices AS device
SET status = 'revoked',
    revoked_at = now()
WHERE device.id = ANY ($1)
  AND device.status = 'active'
  AND device.session_id NOT IN (SELECT alive.session_id FROM alive)
RETURNING device.id, device.user_id
