SELECT conversation.kind,
       conversation.rekey,
       conversation.deleted_at IS NOT NULL AS "deleted!",
       member.state AS "member_state?",
       member.joined_epoch AS "joined_epoch?",
       (conversation.settings ->> 'history_visible') IS DISTINCT FROM 'false' AS "history_visible!",
       EXISTS (
           SELECT 1
           FROM chat.conversation_keys AS stored
           WHERE stored.conversation_id = conversation.id
             AND stored.epoch = conversation.key_epoch
       ) AS "current_key!"
FROM chat.conversations AS conversation
LEFT JOIN chat.members AS member
    ON member.conversation_id = conversation.id
   AND member.user_id = $2
WHERE conversation.id = $1
