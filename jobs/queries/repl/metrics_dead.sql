WITH kinds AS (
    SELECT kind FROM public.scd_kinds()
    UNION
    SELECT kind FROM cmd_dead
)
SELECT kinds.kind AS "kind!",
       count(dead.id)::bigint AS "dead!",
       coalesce(max(extract(epoch FROM now() - dead.at)), 0)::float8 AS "oldest_seconds!"
FROM kinds
LEFT JOIN cmd_dead AS dead ON dead.kind = kinds.kind
GROUP BY kinds.kind
