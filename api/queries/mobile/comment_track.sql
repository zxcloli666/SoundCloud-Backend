SELECT sc_track_id
FROM track_comments
WHERE sc_comment_id = $1
LIMIT 1
