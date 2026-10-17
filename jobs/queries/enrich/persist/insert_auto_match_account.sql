INSERT INTO artist_sc_accounts (artist_id, sc_user_id, role, source, verified)
SELECT $1::uuid, $2::text, 'alt', 'auto_match', false
WHERE NOT EXISTS (SELECT 1
                  FROM artist_sc_accounts AS account
                  WHERE account.artist_id = $1
                    AND (account.verified OR account.source IN ('mb_resolve', 'manual')))
ON CONFLICT (artist_id, sc_user_id) DO NOTHING
