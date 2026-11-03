DELETE FROM diag_issue_origins
WHERE (fingerprint, day, host, origin) IN (
    SELECT fingerprint, day, host, origin
    FROM diag_issue_origins
    WHERE day < (now() AT TIME ZONE 'UTC')::date - 30
    LIMIT $1
)
