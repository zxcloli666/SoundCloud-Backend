SELECT COUNT(session.id)::int8 AS "total!",
       COUNT(session.id) FILTER (WHERE connection.expires_at > now())::int8 AS "valid!",
       COUNT(session.id) FILTER (
           WHERE connection.id IS NULL OR connection.expires_at <= now()
       )::int8 AS "expired!",
       COUNT(session.id) FILTER (
           WHERE connection.expires_at > now()
             AND connection.expires_at <= now() + interval '1 hour'
       )::int8 AS "expiring_1h!",
       COUNT(DISTINCT connection.soundcloud_user_id)::int8 AS "distinct_users!",
       COUNT(session.id) FILTER (
           WHERE session.updated_at > (now() AT TIME ZONE 'utc') - interval '24 hours'
       )::int8 AS "active_24h!"
FROM sessions AS session
LEFT JOIN soundcloud_connections AS connection
    ON connection.id = session.soundcloud_connection_id
