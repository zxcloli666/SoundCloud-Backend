WITH identities AS MATERIALIZED (
    SELECT id,
           btrim(
               client_id,
               U&'\0009\000A\000B\000C\000D\0020\0085\00A0\1680\2000\2001\2002\2003\2004\2005\2006\2007\2008\2009\200A\2028\2029\202F\205F\3000'
           ) AS client_identity
    FROM oauth_apps
), invalid AS MATERIALIZED (
    SELECT client_identity,
           string_agg(id::text, ', ' ORDER BY id) AS app_ids
    FROM identities
    GROUP BY client_identity
    HAVING client_identity = '' OR count(*) > 1
), sample AS (
    SELECT client_identity, app_ids
    FROM invalid
    ORDER BY client_identity
    LIMIT 10
)
SELECT count(*) AS "identity_groups!",
       (
           SELECT string_agg(
                      format('%L => [%s]', client_identity, app_ids),
                      '; ' ORDER BY client_identity
                  )
           FROM sample
       ) AS "sample?"
FROM invalid
