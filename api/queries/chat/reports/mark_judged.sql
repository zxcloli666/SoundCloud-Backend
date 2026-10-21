UPDATE chat.reports
SET handled_by = $2,
    handled_at = now(),
    purge_after = now() + $3::int * interval '1 day'
WHERE id = $1
  AND state = 'open'
  AND handled_by IS NULL
