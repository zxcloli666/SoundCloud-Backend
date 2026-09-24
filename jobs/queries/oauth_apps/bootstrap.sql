WITH input AS MATERIALIZED (
    SELECT btrim(
               $3,
               U&'\0009\000A\000B\000C\000D\0020\0085\00A0\1680\2000\2001\2002\2003\2004\2005\2006\2007\2008\2009\200A\2028\2029\202F\205F\3000'
           ) AS client_id
), existing AS MATERIALIZED (
    SELECT id
    FROM oauth_apps
    CROSS JOIN input
    WHERE btrim(
              oauth_apps.client_id,
              U&'\0009\000A\000B\000C\000D\0020\0085\00A0\1680\2000\2001\2002\2003\2004\2005\2006\2007\2008\2009\200A\2028\2029\202F\205F\3000'
          ) = input.client_id
    ORDER BY active DESC, id
    LIMIT 1
), updated AS (
    UPDATE oauth_apps AS app
    SET name = $2,
        client_id = input.client_id,
        client_secret = $4,
        redirect_uri = $5,
        active = true,
        updated_at = now()
    FROM existing
    CROSS JOIN input
    WHERE app.id = existing.id
    RETURNING app.id
), inserted AS (
    INSERT INTO oauth_apps (
        id,
        name,
        client_id,
        client_secret,
        redirect_uri,
        active
    )
    SELECT $1, $2, input.client_id, $4, $5, true
    FROM input
    WHERE NOT EXISTS (SELECT 1 FROM updated)
    RETURNING id
)
SELECT id AS "id!" FROM updated
UNION ALL
SELECT id FROM inserted
LIMIT 1
