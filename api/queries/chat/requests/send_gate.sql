WITH pending AS (
    SELECT EXISTS (
        SELECT 1
        FROM chat.members m
        JOIN chat.conversations c ON c.id = m.conversation_id
        WHERE m.conversation_id = $1
          AND m.user_id <> $2
          AND m.state IN ('active', 'invited')
          AND NOT m.accepted
          AND c.kind = 'direct'
    ) AS yes
)
SELECT p.yes AS "pending!",
       CASE
           WHEN p.yes THEN (
               SELECT count(*)
               FROM chat.messages
               WHERE conversation_id = $1
                 AND sender_user = $2
                 AND class = 'post'
           )
           ELSE 0
       END AS "sent!",
       u.restricted_until AS "restricted_until?",
       u.restriction_scope AS "restriction_scope?"
FROM pending p
LEFT JOIN chat.users u ON u.user_id = $2 AND u.restricted_until > now()
