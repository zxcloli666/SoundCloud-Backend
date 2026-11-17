UPDATE video_accounts
SET paused_until = now() + make_interval(secs => $2),
    last_error = $3,
    updated_at = now()
WHERE name = $1
