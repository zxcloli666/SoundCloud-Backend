DELETE FROM sc_egress_health
WHERE channel = $1
  AND open_until <= opened_at + INTERVAL '60 seconds'
