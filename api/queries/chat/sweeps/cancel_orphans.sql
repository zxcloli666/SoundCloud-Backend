SELECT count(*) FILTER (WHERE pg_cancel_backend(pid)) AS cancelled
FROM pg_stat_activity
WHERE datname = current_database()
  AND usename = current_user
  AND pid <> pg_backend_pid()
  AND state = 'active'
  AND btrim(query, E' \t\r\n') = ANY($1)
