UPDATE chat.conversations
SET settings = settings || jsonb_build_object('backdrop', $2::jsonb)
WHERE id = $1
