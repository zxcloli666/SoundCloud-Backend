SELECT synced_at
FROM user_collection_sync
WHERE user_id = $1
  AND collection = $2
