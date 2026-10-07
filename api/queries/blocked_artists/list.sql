SELECT DISTINCT ON (kind, target_id) kind,
                                     target_id,
                                     name,
                                     avatar_url,
                                     sc_user_ids,
                                     created_at
FROM user_blocked_artists
WHERE sc_user_id = ANY ($1)
ORDER BY kind, target_id, created_at DESC
