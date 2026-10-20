INSERT INTO chat.conversations (id, kind, dm_key, created_by, meta_ct, meta_epoch)
VALUES ($1, $2, $3, $4, $5, $6)
ON CONFLICT (dm_key) DO NOTHING
RETURNING id
