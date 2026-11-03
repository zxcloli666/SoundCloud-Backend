DELETE FROM diag_issue_days
WHERE (fingerprint, day, host, app_version) IN (
    SELECT fingerprint, day, host, app_version
    FROM diag_issue_days
    WHERE day < (now() AT TIME ZONE 'UTC')::date - 30
    LIMIT $1
)
