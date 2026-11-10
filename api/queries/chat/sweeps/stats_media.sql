WITH found AS (
    SELECT kind, status, count(*) AS objects, sum(bytes) AS bytes
    FROM chat.media
    GROUP BY kind, status
)
SELECT kinds.kind AS "kind!",
       statuses.status AS "status!",
       coalesce(found.objects, 0) AS "objects!",
       coalesce(found.bytes, 0)::bigint AS "bytes!"
FROM unnest($1::text[]) AS kinds (kind)
CROSS JOIN unnest($2::text[]) AS statuses (status)
LEFT JOIN found
    ON found.kind = kinds.kind
   AND found.status = statuses.status
