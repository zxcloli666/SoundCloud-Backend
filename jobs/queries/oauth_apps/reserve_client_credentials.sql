WITH input AS MATERIALIZED (
    SELECT btrim(
               $4,
               U&'\0009\000A\000B\000C\000D\0020\0085\00A0\1680\2000\2001\2002\2003\2004\2005\2006\2007\2008\2009\200A\2028\2029\202F\205F\3000'
           ) AS client_id
), lock AS (
    SELECT pg_advisory_xact_lock($1::bigint)
), cleaned AS (
    DELETE FROM oauth_app_token_issuance_reservations
    WHERE reserved_at < now() - interval '13 hours'
), counts AS (
    SELECT count(*) FILTER (
               WHERE oauth_app_token_issuance_reservations.client_id = input.client_id
                 AND reserved_at > now() - interval '12 hours'
           ) AS app_issued,
           count(*) FILTER (
               WHERE reserved_at > now() - interval '1 hour'
           ) AS egress_issued
    FROM oauth_app_token_issuance_reservations
    CROSS JOIN lock
    CROSS JOIN input
    WHERE released_at IS NULL
), reserved AS (
    INSERT INTO oauth_app_token_issuance_reservations (id, oauth_app_id, client_id)
    SELECT $2, $3, input.client_id
    FROM counts
    CROSS JOIN input
    WHERE app_issued < 45
      AND egress_issued < 27
    RETURNING id
), app_retry AS (
    SELECT min(reserved_at + interval '12 hours') AS retry_at
    FROM oauth_app_token_issuance_reservations
    CROSS JOIN input
    WHERE released_at IS NULL
      AND oauth_app_token_issuance_reservations.client_id = input.client_id
      AND reserved_at > now() - interval '12 hours'
), egress_retry AS (
    SELECT min(reserved_at + interval '1 hour') AS retry_at
    FROM oauth_app_token_issuance_reservations
    WHERE released_at IS NULL
      AND reserved_at > now() - interval '1 hour'
)
SELECT reserved.id AS "reservation_id?",
       CASE
           WHEN reserved.id IS NOT NULL THEN now()
           WHEN counts.app_issued >= 45 THEN app_retry.retry_at
           ELSE egress_retry.retry_at
       END AS "retry_at!"
FROM counts
LEFT JOIN reserved ON true
CROSS JOIN app_retry
CROSS JOIN egress_retry
