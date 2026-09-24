SELECT app.id,
       app.name,
       app.client_id,
       app.active,
       app.last_used_at,
       COUNT(session.id)::int8 AS "sessions_total!",
       COUNT(session.id) FILTER (WHERE connection.expires_at > now())::int8 AS "sessions_active!",
       COUNT(session.id) FILTER (WHERE connection.expires_at <= now())::int8 AS "sessions_expired!"
FROM oauth_apps AS app
LEFT JOIN soundcloud_connections AS connection ON connection.oauth_app_id = app.id
LEFT JOIN sessions AS session ON session.soundcloud_connection_id = connection.id
GROUP BY app.id, app.name, app.client_id, app.active, app.last_used_at
ORDER BY "sessions_total!" DESC, app.name
