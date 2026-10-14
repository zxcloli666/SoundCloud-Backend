INSERT INTO star_track_tiers (sc_track_id, tier)
SELECT DISTINCT liked.sc_track_id, 1::smallint
FROM unnest($2::text[]) AS liked(sc_track_id)
WHERE EXISTS (
    SELECT 1
    FROM subscriptions AS subscription
    WHERE subscription.user_urn = $1
      AND subscription.exp_date > extract(epoch FROM now())::bigint
)
  AND NOT EXISTS (
    SELECT 1
    FROM star_track_tiers AS current
    WHERE current.sc_track_id = liked.sc_track_id
      AND current.tier = 1
)
ON CONFLICT (sc_track_id) DO UPDATE
SET tier = EXCLUDED.tier,
    tiered_at = now()
