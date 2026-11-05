SELECT media.store,
       media.object_key,
       root.status,
       coalesce(root.restorable_until > now(), false) AS "restorable!",
       EXISTS (
           SELECT 1
           FROM chat.media_holds AS hold
           WHERE hold.media_id = root.id
             AND hold.until > now()
       ) AS "held!"
FROM chat.media AS media
JOIN chat.media AS root ON root.id = coalesce(media.parent_id, media.id)
WHERE media.id = $1
