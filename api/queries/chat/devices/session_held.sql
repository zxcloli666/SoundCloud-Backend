SELECT EXISTS (
    SELECT 1
    FROM chat.devices
    WHERE session_id = $1
      AND status = 'active'
) AS "held!"
