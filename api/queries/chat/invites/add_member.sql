INSERT INTO chat.members AS m (conversation_id, user_id, role, state, accepted, joined_seq, joined_epoch, invited_by)
VALUES ($1, $2, 'member', 'active', true, $3, $4, $5)
ON CONFLICT (conversation_id, user_id) DO UPDATE
    SET state = 'active',
        role = 'member',
        accepted = true,
        joined_seq = EXCLUDED.joined_seq,
        joined_epoch = EXCLUDED.joined_epoch,
        invited_by = EXCLUDED.invited_by,
        updated_at = now()
    WHERE m.state IN ('left', 'kicked', 'invited')
RETURNING conversation_id
