SELECT EXISTS (
    SELECT 1
    FROM chat.devices
    WHERE session_hash = $1
      AND status = 'active'
) AS "held!"
