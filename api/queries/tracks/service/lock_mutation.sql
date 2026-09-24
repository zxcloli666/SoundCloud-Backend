SELECT 1 AS "locked!"
FROM pg_advisory_xact_lock(hashtextextended('track-mutation:' || $1::text, 0))
