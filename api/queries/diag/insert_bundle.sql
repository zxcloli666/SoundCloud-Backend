INSERT INTO diag_bundles (code, app_version, os, body)
VALUES ($1, $2, $3, $4)
ON CONFLICT (code) DO NOTHING
RETURNING at AS "at!"
