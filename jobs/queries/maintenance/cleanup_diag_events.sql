DELETE FROM diag_events
WHERE id IN (
    SELECT id
    FROM diag_events
    WHERE at < now() - interval '7 days'
    LIMIT $1
)
