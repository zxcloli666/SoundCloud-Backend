WITH RECURSIVE stores AS (
    (SELECT store FROM chat.media ORDER BY store LIMIT 1)
    UNION ALL
    SELECT (SELECT next.store FROM chat.media AS next WHERE next.store > stores.store ORDER BY next.store LIMIT 1)
    FROM stores
    WHERE stores.store IS NOT NULL
)
SELECT store AS "store!"
FROM stores
WHERE store IS NOT NULL
