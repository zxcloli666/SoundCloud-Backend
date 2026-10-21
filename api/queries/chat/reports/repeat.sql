SELECT EXISTS (
    SELECT 1
    FROM chat.reports
    WHERE target_user = $1
      AND id <> $2
      AND state = 'actioned'
      AND handled_at > now() - $3::int * interval '1 day'
      AND NOT (conversation_id = $4 AND seq_from <= $6 AND seq_to >= $5)
) AS "repeat!"
