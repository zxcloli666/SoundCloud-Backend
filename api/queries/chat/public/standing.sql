SELECT m.joined_at AS "joined_at?",
       (SELECT p.until
        FROM chat.public_mutes p
        WHERE p.user_id = m.user_id
          AND p.until > now()) AS "muted_until?"
FROM chat.members m
WHERE m.conversation_id = $1
  AND m.user_id = $2
