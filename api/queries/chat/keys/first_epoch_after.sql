SELECT COALESCE(
           (
               SELECT min(stored.epoch)
               FROM chat.conversation_keys AS stored
               WHERE stored.conversation_id = conversation.id
                 AND stored.created_at > $2
           ),
           conversation.key_epoch
       ) AS "epoch!"
FROM chat.conversations AS conversation
WHERE conversation.id = $1
