SELECT NOT EXISTS (SELECT 1
                   FROM artist_sc_accounts AS account
                   WHERE account.artist_id = artist.id
                     AND account.sc_user_id = $4
                     AND (account.verified OR account.source IN ('mb_resolve', 'manual')))
           AND (COALESCE(artist.mb_artist_id <> $2, false)
               OR COALESCE(artist.genius_artist_id <> $3, false)
               OR EXISTS (SELECT 1
                          FROM artist_sc_accounts AS account
                          WHERE account.artist_id = artist.id
                            AND (account.verified OR account.source IN ('mb_resolve', 'manual')))) AS "another!"
FROM artists AS artist
WHERE artist.id = $1
