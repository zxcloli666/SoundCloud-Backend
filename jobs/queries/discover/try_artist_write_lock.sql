SELECT pg_try_advisory_xact_lock(
    hashtextextended('jobs:discover.artist_writes', 0)
) AS "acquired!"
