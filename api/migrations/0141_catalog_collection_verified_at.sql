ALTER TABLE catalog_collection_sync
    ADD COLUMN IF NOT EXISTS verified_at timestamptz;

UPDATE catalog_collection_sync
SET verified_at = synced_at
WHERE scope = 'owner' AND synced_at IS NOT NULL AND verified_at IS NULL;
