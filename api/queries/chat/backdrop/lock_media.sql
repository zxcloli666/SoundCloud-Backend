SELECT media.owner_user,
       media.kind,
       media.status,
       media.origin_conversation
FROM chat.media AS media
JOIN chat.conversations AS conversation
    ON conversation.id = media.origin_conversation
WHERE media.id = $1
  AND conversation.deleted_at IS NULL
FOR SHARE OF media
