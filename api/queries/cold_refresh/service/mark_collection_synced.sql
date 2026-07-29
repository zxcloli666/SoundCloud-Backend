INSERT INTO user_collection_sync (user_id, collection, synced_at)
VALUES ($1, $2, $3)
ON CONFLICT (user_id, collection) DO UPDATE SET synced_at = EXCLUDED.synced_at
WHERE user_collection_sync.synced_at < EXCLUDED.synced_at
