INSERT INTO chat.members AS m (conversation_id, user_id, role, state, accepted, joined_seq, joined_epoch, invited_by)
SELECT $1,
       joining.user_id,
       joining.role,
       joining.state,
       joining.accepted,
       $2,
       $3,
       NULLIF($4, joining.user_id)
FROM unnest($5::text[], $6::text[], $7::text[], $8::boolean[]) AS joining(user_id, role, state, accepted)
ON CONFLICT (conversation_id, user_id) DO UPDATE
SET role = EXCLUDED.role,
    state = EXCLUDED.state,
    accepted = EXCLUDED.accepted,
    joined_seq = EXCLUDED.joined_seq,
    joined_epoch = EXCLUDED.joined_epoch,
    invited_by = EXCLUDED.invited_by,
    updated_at = now()
WHERE m.state IN ('left', 'kicked')
RETURNING m.user_id, m.state
