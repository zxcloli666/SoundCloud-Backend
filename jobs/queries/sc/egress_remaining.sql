SELECT GREATEST(0, EXTRACT(EPOCH FROM (open_until - now())))::double precision AS "remaining!"
FROM sc_egress_health
WHERE channel = $1
