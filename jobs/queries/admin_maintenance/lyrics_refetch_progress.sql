SELECT count(*) AS "armed!",
       count(*) FILTER (WHERE status = 'pending') AS "pending!",
       count(*) FILTER (WHERE outcome = 'replaced') AS "replaced!",
       count(*) FILTER (
           WHERE outcome IN ('unchanged', 'different', 'not_found', 'stale', 'no_metadata')
       ) AS "kept!",
       count(*) FILTER (WHERE outcome IN ('failed', 'abandoned')) AS "lost!"
FROM lyrics_repair
WHERE kind = 'refetch'
