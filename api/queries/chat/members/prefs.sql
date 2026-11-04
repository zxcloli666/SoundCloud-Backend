UPDATE chat.members
SET muted_until = CASE WHEN $3 THEN $4 ELSE muted_until END,
    notify = COALESCE($5, notify),
    pinned_at = CASE
                    WHEN $6::boolean IS NULL THEN pinned_at
                    WHEN $6 THEN COALESCE(pinned_at, now())
                    ELSE NULL
                    END,
    archived = COALESCE($7, archived),
    updated_at = now()
WHERE conversation_id = $1
  AND user_id = $2
  AND state IN ('active', 'invited')
RETURNING user_id
