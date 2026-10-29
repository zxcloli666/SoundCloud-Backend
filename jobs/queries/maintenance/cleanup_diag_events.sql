DELETE FROM diag_events
WHERE id IN (
    SELECT id
    FROM diag_events
    WHERE at < now() - interval '30 days'
    LIMIT $1
)
