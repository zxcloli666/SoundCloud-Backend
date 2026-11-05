SELECT bytes,
       reserved,
       CASE WHEN day = current_date THEN day_bytes ELSE 0 END AS "day_bytes!"
FROM chat.usage
WHERE user_id = $1
