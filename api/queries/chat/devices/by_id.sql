SELECT id,
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
FROM chat.devices
WHERE id = $1
