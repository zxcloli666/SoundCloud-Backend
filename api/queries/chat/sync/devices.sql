SELECT
    id,
    platform,
    name,
    app_version,
    tier,
    created_at,
    last_seen_at,
    history_unlocked_at IS NOT NULL AS "history_unlocked!"
FROM chat.devices
WHERE user_id = $1
  AND status = 'active'
ORDER BY created_at, id
