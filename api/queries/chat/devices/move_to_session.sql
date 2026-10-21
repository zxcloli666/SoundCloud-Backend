UPDATE chat.devices
SET session_id = $2,
    platform = $3,
    name = $4,
    app_version = $5,
    caps = $6,
    last_seen_at = now()
WHERE id = $1
  AND status = 'active'
RETURNING id,
          user_id,
          session_id,
          status,
          auth_pub,
          box_pub,
          platform,
          name,
          app_version,
          caps,
          tier,
          created_at,
          history_unlocked_at,
          last_seen_at,
          revoked_at
