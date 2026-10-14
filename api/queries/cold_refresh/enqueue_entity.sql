INSERT INTO background_jobs (id, kind, lane, dedup_key, payload, priority, max_attempts)
VALUES (
    $1,
    $2,
    $3,
    $4,
    $5,
    $6::smallint + CASE
        WHEN EXISTS (
            SELECT 1
            FROM subscriptions AS subscription
            WHERE subscription.user_urn = $7
              AND subscription.exp_date > extract(epoch FROM now())::bigint
        ) THEN $8::smallint
        ELSE 0::smallint
    END,
    8
)
ON CONFLICT (kind, dedup_key) WHERE dedup_key IS NOT NULL DO UPDATE
SET priority = EXCLUDED.priority
WHERE background_jobs.priority < EXCLUDED.priority
