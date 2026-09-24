SELECT user_urn, exp_date
FROM subscriptions
ORDER BY user_urn
LIMIT $1
