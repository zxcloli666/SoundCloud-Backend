SELECT account.artist_id,
       account.sc_user_id
FROM artist_sc_accounts AS account
WHERE NOT account.verified
  AND account.source <> 'mb_resolve'
  AND ($1::uuid IS NULL
    OR (account.artist_id, account.sc_user_id) > ($1::uuid, $2::text))
ORDER BY account.artist_id, account.sc_user_id
LIMIT 1
