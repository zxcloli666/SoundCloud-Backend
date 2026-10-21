SELECT COALESCE(
           (
               SELECT message.epoch
               FROM chat.messages AS message
               WHERE message.conversation_id = conversation.id
                 AND message.seq > $2
               ORDER BY message.seq
               LIMIT 1
           ),
           conversation.key_epoch
       ) AS "epoch!"
FROM chat.conversations AS conversation
WHERE conversation.id = $1
