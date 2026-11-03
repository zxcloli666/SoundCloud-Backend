INSERT INTO diag_issue_origins (fingerprint, day, host, origin)
VALUES ($1, (now() AT TIME ZONE 'UTC')::date, $2, $3)
ON CONFLICT DO NOTHING
