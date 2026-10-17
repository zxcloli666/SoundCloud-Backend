SELECT EXISTS (SELECT 1
               FROM artist_sc_accounts AS account
               WHERE account.artist_id = artist.id
                 AND (account.verified OR account.source IN ('mb_resolve', 'manual')))
           AND NOT EXISTS (SELECT 1
                           FROM artist_sc_accounts AS account
                           WHERE account.artist_id = artist.id
                             AND account.sc_user_id = $2
                             AND (account.verified OR account.source IN ('mb_resolve', 'manual'))) AS "held!"
FROM artists AS artist
WHERE artist.normalized_name = $1
  AND artist.merged_into IS NULL
