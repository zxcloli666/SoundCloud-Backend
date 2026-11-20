SELECT sync.synced_at AS "synced_at?",
       sync.complete AS "complete?",
       sync.page_count AS "page_count?",
       sync.item_count AS "item_count?",
       EXISTS (
           SELECT 1
           FROM job_status AS status
           WHERE status.kind = 'catalog.collection' AND status.dedup_key = $4
       ) AS "queued!"
FROM (VALUES (1)) AS one (n)
LEFT JOIN catalog_collection_sync AS sync
  ON sync.subject_id = $1 AND sync.collection = $2 AND sync.scope = $3
