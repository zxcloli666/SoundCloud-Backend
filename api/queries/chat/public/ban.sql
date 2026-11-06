INSERT INTO chat.members (conversation_id, user_id, role, state, accepted, notify, joined_at)
VALUES ($1, $2, 'member', 'banned', true, 'mentions', now())
ON CONFLICT (conversation_id, user_id) DO UPDATE
SET state = 'banned',
    role = 'member',
    updated_at = now()
