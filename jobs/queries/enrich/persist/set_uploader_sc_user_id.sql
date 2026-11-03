UPDATE tracks
SET uploader_sc_user_id = COALESCE(uploader_sc_user_id, $2)
WHERE id = $1
  AND uploader_sc_user_id IS DISTINCT FROM COALESCE(uploader_sc_user_id, $2)
