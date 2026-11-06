UPDATE chat.devices
SET app_version = $2,
    caps = $3
WHERE id = $1
  AND status = 'active'
  AND (app_version IS DISTINCT FROM $2 OR caps IS DISTINCT FROM $3)
RETURNING id,
          user_id,
          session_hash,
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
