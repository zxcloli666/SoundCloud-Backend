SELECT store, coalesce(sum(bytes), 0)::bigint AS "bytes!"
FROM chat.media
GROUP BY store
