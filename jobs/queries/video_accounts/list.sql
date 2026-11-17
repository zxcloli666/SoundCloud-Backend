SELECT
    name,
    source,
    status,
    last_checked_at,
    last_error,
    left(encode(sha256(convert_to(secret, 'UTF8')), 'hex'), 12) AS "secret_sha256!"
FROM video_accounts
ORDER BY source, name
