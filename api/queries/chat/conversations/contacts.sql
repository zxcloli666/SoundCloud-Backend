SELECT candidate AS "user_id!",
       EXISTS (SELECT 1
               FROM user_followings f
               WHERE f.user_id IN (candidate, 'soundcloud:users:' || candidate)
                 AND f.target_user_urn = 'soundcloud:users:' || $1
                 AND f.wanted_state) AS "follows!",
       EXISTS (SELECT 1
               FROM chat.members mine
               JOIN chat.members theirs
                 ON theirs.conversation_id = mine.conversation_id
                AND theirs.user_id = candidate
               JOIN chat.conversations c ON c.id = mine.conversation_id
               WHERE mine.user_id = $1
                 AND mine.state = 'active'
                 AND mine.accepted
                 AND theirs.state = 'active'
                 AND theirs.accepted
                 AND c.kind IN ('direct', 'group')
                 AND c.deleted_at IS NULL) AS "contact!"
FROM unnest($2::text[]) AS candidate
