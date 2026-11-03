INSERT INTO subscriptions (user_urn, exp_date, star_since)
VALUES ($1, $2, $3) ON CONFLICT (user_urn) DO
UPDATE SET exp_date = EXCLUDED.exp_date, star_since = EXCLUDED.star_since
