SELECT c.user_urn AS "author!",
       t.uploader_sc_user_id,
       t.uploader_urn
FROM track_comments c
LEFT JOIN tracks t ON t.sc_track_id = c.sc_track_id
WHERE c.sc_comment_id = $1
LIMIT 1
