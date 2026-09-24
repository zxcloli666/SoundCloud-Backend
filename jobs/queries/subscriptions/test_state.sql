SELECT COUNT(*)::int8 AS "count!",
       EXISTS (
           SELECT 1
           FROM subscription_snapshot_state
           WHERE singleton = true
       ) AS "initialized!"
FROM subscriptions
