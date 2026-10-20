WITH recent AS (
    SELECT reporter, conversation_id, weight
    FROM chat.reports
    WHERE target_user = $1
      AND state <> 'rejected'
      AND weight > 0
      AND created_at > now() - $2::int * interval '1 hour'
)
SELECT COALESCE((
           SELECT sum(heaviest)
           FROM (SELECT max(weight) AS heaviest FROM recent GROUP BY reporter) AS per_reporter
       ), 0)::real AS "weight!",
       (SELECT count(DISTINCT conversation_id) FROM recent)::int AS "conversations!"
