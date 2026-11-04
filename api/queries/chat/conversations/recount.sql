UPDATE chat.conversations
SET member_count = (SELECT count(*)
                    FROM chat.members
                    WHERE conversation_id = $1
                      AND state = 'active')
WHERE id = $1
RETURNING member_count
