SELECT (
           SELECT coalesce(
                      artist.last_account_walk_at + $1::bigint * interval '1 day',
                      artist.created_at
                  )
           FROM artists AS artist
           WHERE artist.is_star
             AND artist.merged_into IS NULL
             AND (
                 artist.last_account_walk_at IS NULL
                 OR artist.last_account_walk_at < now() - $1::bigint * interval '1 day'
             )
             AND (
                 artist.account_walk_locked_at IS NULL
                 OR artist.account_walk_locked_at < now() - $2::bigint * interval '1 second'
             )
             AND artist.has_sc_account
           ORDER BY artist.last_account_walk_at NULLS FIRST
           LIMIT 1
       ) AS star,
       (
           SELECT coalesce(
                      artist.last_account_walk_at + $1::bigint * interval '1 day',
                      artist.created_at
                  )
           FROM artists AS artist
           WHERE artist.merged_into IS NULL
             AND (
                 artist.last_account_walk_at IS NULL
                 OR artist.last_account_walk_at < now() - $1::bigint * interval '1 day'
             )
             AND (
                 artist.account_walk_locked_at IS NULL
                 OR artist.account_walk_locked_at < now() - $2::bigint * interval '1 second'
             )
             AND artist.has_sc_account
             AND NOT artist.is_star
           ORDER BY artist.last_account_walk_at NULLS FIRST
           LIMIT 1
       ) AS regular
