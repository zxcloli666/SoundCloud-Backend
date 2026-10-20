SELECT EXISTS (
    SELECT 1
    FROM chat.reports
    WHERE target_user = $1
      AND id <> $2
      AND state = 'actioned'
      AND handled_at > now() - $3::int * interval '1 day'
) AS "repeat!"
