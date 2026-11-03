SELECT exp_date, star_since
FROM subscriptions
WHERE user_urn = ANY ($1)
ORDER BY exp_date DESC
LIMIT 1
