SELECT 1 AS "locked!"
FROM pg_advisory_xact_lock(hashtextextended('history-play:' || $1::text || ':' || $2::text, 0))
