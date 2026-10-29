WITH never_walked AS (
    SELECT artist.id, artist.last_account_walk_at
    FROM artists AS artist
    WHERE artist.is_star
      AND artist.merged_into IS NULL
      AND artist.last_account_walk_at IS NULL
      AND (
          artist.account_walk_locked_at IS NULL
          OR artist.account_walk_locked_at < now() - $2::bigint * interval '1 second'
      )
      AND artist.has_sc_account
    ORDER BY artist.last_account_walk_at NULLS FIRST
    LIMIT $3
    FOR UPDATE SKIP LOCKED
), walked_long_ago AS (
    SELECT artist.id, artist.last_account_walk_at
    FROM artists AS artist
    WHERE artist.is_star
      AND artist.merged_into IS NULL
      AND artist.last_account_walk_at < now() - $1::bigint * interval '1 day'
      AND (
          artist.account_walk_locked_at IS NULL
          OR artist.account_walk_locked_at < now() - $2::bigint * interval '1 second'
      )
      AND artist.has_sc_account
    ORDER BY artist.last_account_walk_at NULLS FIRST
    LIMIT $3
    FOR UPDATE SKIP LOCKED
), picked AS (
    SELECT candidate.id
    FROM (
        SELECT id, last_account_walk_at FROM never_walked
        UNION ALL
        SELECT id, last_account_walk_at FROM walked_long_ago
    ) AS candidate
    ORDER BY candidate.last_account_walk_at NULLS FIRST
    LIMIT $3
)
UPDATE artists AS artist
SET account_walk_locked_at = now()
FROM picked
WHERE artist.id = picked.id
RETURNING artist.id
