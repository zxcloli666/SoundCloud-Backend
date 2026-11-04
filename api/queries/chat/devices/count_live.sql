SELECT count(*) AS "live!"
FROM chat.devices
WHERE user_id = $1
  AND status = 'active'
