INSERT INTO chat.members AS m (conversation_id, user_id, role, state, accepted, notify, joined_at)
VALUES ($1, $2, $3, 'active', true, 'mentions', now())
ON CONFLICT (conversation_id, user_id) DO UPDATE
SET role = EXCLUDED.role,
    updated_at = now()
WHERE m.state = 'active'
  AND m.role <> 'owner'
RETURNING m.role
