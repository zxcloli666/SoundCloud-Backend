UPDATE catalog_collection_sync
SET next_cursor = $4,
    page_count = page_count + 1,
    item_count = item_count + $5,
    complete = $6,
    synced_at = CASE WHEN $6 THEN now() ELSE synced_at END,
    updated_at = now()
WHERE subject_id = $1 AND collection = $2 AND scope = $3
