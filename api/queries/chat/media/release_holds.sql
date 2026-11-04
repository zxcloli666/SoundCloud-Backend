DELETE FROM chat.media_holds
WHERE report_id = $1
RETURNING media_id
