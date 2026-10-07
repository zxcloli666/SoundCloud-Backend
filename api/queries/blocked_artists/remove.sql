DELETE
FROM user_blocked_artists
WHERE sc_user_id = ANY ($1)
  AND kind = $2
  AND target_id = $3
