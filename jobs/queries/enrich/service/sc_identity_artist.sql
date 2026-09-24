SELECT artist.id,
       artist.name,
       artist.mb_artist_id,
       artist.genius_artist_id,
       account.verified
FROM artist_sc_accounts AS account
         JOIN artists AS artist ON artist.id = account.artist_id
WHERE account.sc_user_id = $1
  AND artist.merged_into IS NULL
  AND (account.verified OR account.source = 'mb_resolve')
ORDER BY account.verified DESC, artist.id
LIMIT 2
