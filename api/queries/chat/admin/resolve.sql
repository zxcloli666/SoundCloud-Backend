UPDATE chat.reports
SET state = $2,
    handled_by = $3,
    handled_at = now(),
    purge_after = CASE WHEN $2 = 'open' THEN NULL ELSE now() + $4::int * interval '1 day' END
WHERE id = $1
RETURNING id
