INSERT INTO chat.media_refs (media_id, conversation_id, seq)
SELECT media_id, $2::uuid, $3::bigint
FROM unnest($1::uuid[]) AS media_id
ON CONFLICT DO NOTHING
