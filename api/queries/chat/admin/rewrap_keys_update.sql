UPDATE chat.conversation_keys AS k
SET wrapped = fresh.wrapped,
    kek_version = $4
FROM unnest($1::uuid[], $2::int[], $3::bytea[]) AS fresh (conversation_id, epoch, wrapped)
WHERE k.conversation_id = fresh.conversation_id
  AND k.epoch = fresh.epoch
  AND k.kek_version <> $4
