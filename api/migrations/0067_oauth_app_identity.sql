SET LOCAL lock_timeout = '5s';

DO $migration$
DECLARE
    invalid_groups bigint;
    invalid_sample text;
BEGIN
    WITH identities AS MATERIALIZED (
        SELECT id,
               btrim(
                   client_id,
                   U&'\0009\000A\000B\000C\000D\0020\0085\00A0\1680\2000\2001\2002\2003\2004\2005\2006\2007\2008\2009\200A\2028\2029\202F\205F\3000'
               ) AS client_identity
        FROM oauth_apps
    ), invalid AS (
        SELECT client_identity,
               string_agg(id::text, ', ' ORDER BY id) AS app_ids
        FROM identities
        GROUP BY client_identity
        HAVING client_identity = '' OR count(*) > 1
    )
    SELECT count(*),
           (
               SELECT string_agg(
                          format('%L => [%s]', client_identity, app_ids),
                          '; ' ORDER BY client_identity
                      )
               FROM (
                   SELECT client_identity, app_ids
                   FROM invalid
                   ORDER BY client_identity
                   LIMIT 10
               ) AS sample
           )
    INTO invalid_groups, invalid_sample
    FROM invalid;

    IF invalid_groups > 0 THEN
        RAISE EXCEPTION USING
            ERRCODE = '23505',
            MESSAGE = format(
                'migration 0067 refused %s invalid OAuth client identities',
                invalid_groups
            ),
            DETAIL = invalid_sample,
            HINT = 'Remove blank client IDs, choose one canonical oauth_apps row per identity, and remap every dependency before retrying';
    END IF;
END
$migration$;

CREATE UNIQUE INDEX oauth_apps_client_identity_uq
    ON oauth_apps (
        btrim(
            client_id,
            U&'\0009\000A\000B\000C\000D\0020\0085\00A0\1680\2000\2001\2002\2003\2004\2005\2006\2007\2008\2009\200A\2028\2029\202F\205F\3000'
        )
    );
