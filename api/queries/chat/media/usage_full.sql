SELECT bytes,
       reserved,
       files
FROM chat.usage
WHERE user_id = $1
