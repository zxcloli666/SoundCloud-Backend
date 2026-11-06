UPDATE chat.conversations
SET settings = jsonb_set(settings, '{slow_mode_s}', to_jsonb($2::int))
WHERE id = $1
  AND kind = 'public'
RETURNING last_seq
