WITH old AS (
    SELECT id, meta_ct, meta_epoch
    FROM chat.conversations
    WHERE id = $1
      AND kind = 'room'
    FOR UPDATE
)
UPDATE chat.conversations AS conversation
SET meta_ct = NULL,
    meta_epoch = NULL
FROM old
WHERE conversation.id = old.id
RETURNING old.meta_ct AS "meta_ct?", old.meta_epoch AS "meta_epoch?"
