UPDATE chat.reports
SET state = $2,
    handled_by = $3,
    handled_at = now(),
    purge_after = now() + $4::int * interval '1 day'
WHERE id = $1
RETURNING id
