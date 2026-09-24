SELECT EXISTS (
    SELECT 1
    FROM subscription_snapshot_state
    WHERE singleton = true
) AS "initialized!",
EXISTS (
    SELECT 1
    FROM subscriptions
) AS "has_subscriptions!"
