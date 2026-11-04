SELECT id,
       reporter,
       target_user,
       conversation_id,
       seq_from,
       seq_to,
       reason,
       comment,
       weight,
       state,
       created_at,
       handled_by,
       handled_at,
       (evidence IS NOT NULL) AS "has_evidence!"
FROM chat.reports
WHERE ($1::text = 'all' OR state = $1)
  AND created_at < $2
  AND ($4::uuid IS NULL OR (created_at, id) < (SELECT c.created_at, c.id FROM chat.reports c WHERE c.id = $4))
ORDER BY created_at DESC, id DESC
LIMIT $3
