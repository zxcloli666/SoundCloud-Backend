WITH ranked AS (
    SELECT account.artist_id,
           aura.aura_id,
           aura.custom_hex,
           row_number() OVER (
               PARTITION BY account.artist_id
               ORDER BY account.verified DESC,
                        CASE account.role WHEN 'main' THEN 0 WHEN 'demo' THEN 1 ELSE 2 END,
                        account.sc_user_id
           ) AS position
    FROM artist_sc_accounts AS account
    LEFT JOIN user_auras AS aura
        ON aura.user_urn = 'soundcloud:users:' || account.sc_user_id
    WHERE account.role IN ('main', 'demo')
), aggregate AS (
    SELECT artist.id,
           ranked.artist_id IS NOT NULL AS is_star,
           ranked.aura_id,
           ranked.custom_hex
    FROM artists AS artist
    LEFT JOIN ranked
        ON ranked.artist_id = artist.id
       AND ranked.position = 1
    WHERE artist.merged_into IS NULL
)
UPDATE artists AS artist
SET is_star = aggregate.is_star,
    star_aura_id = aggregate.aura_id,
    star_custom_hex = aggregate.custom_hex
FROM aggregate
WHERE artist.id = aggregate.id
  AND (artist.is_star, artist.star_aura_id, artist.star_custom_hex)
      IS DISTINCT FROM (aggregate.is_star, aggregate.aura_id, aggregate.custom_hex)
