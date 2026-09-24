WITH marked AS (
    UPDATE soundcloud_connections AS connection
    SET refresh_failure_count = refresh_failure_count + 1,
        last_refresh_error_kind = 'token_rejected',
        last_refresh_error = 'SoundCloud rejected the refreshed access token',
        retry_at = GREATEST(connection.retry_at, now() + $3::int * interval '1 second'),
        updated_at = now()
    FROM sessions AS session
    WHERE session.id = $1
      AND session.soundcloud_connection_id = connection.id
      AND connection.access_token = $2
      AND connection.last_refresh_error_kind IS DISTINCT FROM 'token_rejected'
      AND connection.last_refresh_error_kind IS DISTINCT FROM 'reauthorization_required'
      AND (connection.refresh_lease_id IS NULL OR connection.refresh_lease_expires_at <= now())
    RETURNING connection.retry_at
)
SELECT GREATEST(
           1,
           ceil(EXTRACT(EPOCH FROM (marked.retry_at - now())))::bigint
       ) AS "retry_after_sec!"
FROM marked
UNION ALL
SELECT GREATEST(
           1,
           ceil(EXTRACT(EPOCH FROM (connection.retry_at - now())))::bigint
       ) AS "retry_after_sec!"
FROM soundcloud_connections AS connection
JOIN sessions AS session ON session.soundcloud_connection_id = connection.id
WHERE session.id = $1
  AND connection.access_token = $2
  AND connection.last_refresh_error_kind = 'token_rejected'
  AND connection.retry_at IS NOT NULL
  AND NOT EXISTS (SELECT 1 FROM marked)
LIMIT 1
