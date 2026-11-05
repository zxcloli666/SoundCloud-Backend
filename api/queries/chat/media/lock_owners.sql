SELECT usage.user_id
FROM chat.usage AS usage
WHERE usage.user_id IN (
    SELECT media.owner_user
    FROM chat.media AS media
    WHERE media.id = ANY($1::uuid[])
)
ORDER BY usage.user_id
FOR UPDATE OF usage
