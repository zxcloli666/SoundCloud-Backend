SELECT owner_user,
       kind,
       status,
       origin_conversation
FROM chat.media
WHERE id = $1
FOR NO KEY UPDATE
