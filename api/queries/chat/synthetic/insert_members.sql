INSERT INTO chat.members (conversation_id, user_id, role, state, accepted)
SELECT member.conversation_id, member.user_id, member.role, 'active', true
FROM unnest($1::uuid[], $2::text[], $3::text[]) AS member(conversation_id, user_id, role)
ON CONFLICT (conversation_id, user_id) DO NOTHING
