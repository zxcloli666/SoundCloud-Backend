SELECT at AS "at!",
       app_version AS "app_version!",
       os AS "os!",
       trace,
       body -> 'context' AS "context"
FROM diag_events
WHERE fingerprint = $1
ORDER BY at DESC
LIMIT 20
