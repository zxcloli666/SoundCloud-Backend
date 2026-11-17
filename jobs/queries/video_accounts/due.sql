SELECT name
FROM video_accounts
WHERE last_checked_at IS NULL
   OR last_checked_at < now() - make_interval(secs => $1)
ORDER BY last_checked_at NULLS FIRST, name
