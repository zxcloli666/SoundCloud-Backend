UPDATE chat.members AS member
SET last_read_seq = mark.seq
FROM UNNEST($1::uuid[], $2::text[], $3::int8[]) AS mark (conversation_id, user_id, seq)
WHERE member.conversation_id = mark.conversation_id
  AND member.user_id = mark.user_id
  AND member.last_read_seq < mark.seq
