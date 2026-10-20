UPDATE chat.conversations
SET last_seq = last_seq + 1,
    last_at = now(),
    epoch_messages = epoch_messages + 1,
    meta_ct = CASE WHEN $3 = 'meta' THEN $4 ELSE meta_ct END,
    meta_epoch = CASE WHEN $3 = 'meta' THEN $2 ELSE meta_epoch END
WHERE id = $1
  AND deleted_at IS NULL
  AND key_epoch = $2
  AND NOT rekey
  AND epoch_messages < $5
  AND epoch_started_at > now() - $6::bigint * interval '1 day'
RETURNING last_seq, ttl_s, now() AS "at!"
