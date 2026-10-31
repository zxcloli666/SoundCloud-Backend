SELECT 1 AS "locked!"
FROM pg_advisory_xact_lock(hashtextextended('admin-catalog-request:' || $1::text, 0))
