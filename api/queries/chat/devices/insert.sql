INSERT INTO chat.devices (id, user_id, session_id, auth_pub, box_pub, platform, name, app_version, caps, tier)
VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
ON CONFLICT (id) DO NOTHING
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
