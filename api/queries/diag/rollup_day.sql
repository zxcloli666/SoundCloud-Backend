INSERT INTO diag_issue_days (fingerprint, day, host, app_version, kind, title, events, first_at, last_at)
VALUES ($1, (now() AT TIME ZONE 'UTC')::date, $2, $3, $4, $5, 1, now(), now())
ON CONFLICT (fingerprint, day, host, app_version)
DO UPDATE SET events = diag_issue_days.events + 1, last_at = now()
