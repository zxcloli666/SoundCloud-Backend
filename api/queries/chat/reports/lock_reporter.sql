SELECT 1 AS "locked!"
FROM pg_advisory_xact_lock(hashtextextended('backdrop-report:' || $1::uuid::text || ':' || $2::text, 0))
