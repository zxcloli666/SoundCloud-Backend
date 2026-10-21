INSERT INTO chat.devices (id, user_id, session_id, auth_pub, box_pub, platform, name, app_version, tier, history_unlocked_at)
SELECT device.id,
       device.user_id,
       device.session_id,
       device.auth_pub,
       device.box_pub,
       'loadtest',
       'lt',
       '0.0.0',
       'direct',
       now()
FROM unnest($1::uuid[], $2::text[], $3::uuid[], $4::bytea[], $5::bytea[]) AS device(id, user_id, session_id, auth_pub, box_pub)
ON CONFLICT (id) DO NOTHING
