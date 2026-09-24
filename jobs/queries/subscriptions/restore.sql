INSERT INTO subscriptions (user_urn, exp_date)
SELECT user_urn, exp_date
FROM UNNEST($1::text[], $2::int8[]) AS restored(user_urn, exp_date)
ON CONFLICT (user_urn) DO UPDATE
SET exp_date = excluded.exp_date
