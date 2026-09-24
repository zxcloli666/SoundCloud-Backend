INSERT INTO oauth_app_request_cooldowns (
    oauth_app_id,
    retry_at,
    failure_count
)
VALUES (
    $1,
    now() + GREATEST(1, $2::bigint) * interval '1 second',
    1
)
ON CONFLICT (oauth_app_id) DO UPDATE
SET failure_count = CASE
        WHEN oauth_app_request_cooldowns.retry_at > now()
            THEN LEAST(oauth_app_request_cooldowns.failure_count + 1, 31)
        ELSE 1
    END,
    retry_at = GREATEST(
        oauth_app_request_cooldowns.retry_at,
        now() + GREATEST(1, $2::bigint) * interval '1 second'
    ),
    updated_at = now()
RETURNING GREATEST(
    1,
    ceil(EXTRACT(EPOCH FROM (retry_at - now())))::bigint
) AS "retry_after_seconds!"
