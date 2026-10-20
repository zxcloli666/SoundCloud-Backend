UPDATE chat.reports
SET handled_by = $2,
    handled_at = now()
WHERE id = $1
  AND state = 'open'
  AND handled_by IS NULL
