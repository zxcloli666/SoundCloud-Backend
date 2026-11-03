DELETE FROM diag_bundles
WHERE code IN (
    SELECT code
    FROM diag_bundles
    WHERE at < now() - interval '30 days'
    LIMIT $1
)
