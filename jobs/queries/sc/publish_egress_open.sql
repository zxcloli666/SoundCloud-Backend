INSERT INTO sc_egress_health (channel, open_until, opened_by, opened_at)
VALUES ($1, now() + ($3::double precision * INTERVAL '1 second'), $2, now())
ON CONFLICT (channel) DO UPDATE
SET open_until = GREATEST(sc_egress_health.open_until, EXCLUDED.open_until),
    opened_by = EXCLUDED.opened_by,
    opened_at = EXCLUDED.opened_at
