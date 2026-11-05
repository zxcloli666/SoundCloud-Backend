SELECT bytes, reserved
FROM chat.usage
WHERE user_id = $1
