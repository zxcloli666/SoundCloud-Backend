SELECT sc_user_id,
       urn,
       username,
       username_normalized,
       full_name,
       first_name,
       last_name,
       permalink,
       permalink_url,
       avatar_url,
       country,
       city,
       description,
       verified,
       followers_count,
       followings_count,
       tracks_count,
       playlists_count,
       reposts_count,
       comments_count,
       kind,
       sc_created_at,
       sc_last_modified,
       sc_synced_at,
       last_read_at,
       created_at,
       updated_at
FROM users
WHERE ($1::text IS NULL OR username_normalized LIKE $5
   OR (NOT $6::bool AND LOWER(username) LIKE $1))
  AND ($4::text[] IS NULL OR sc_user_id = ANY($4))
ORDER BY array_position($4::text[], sc_user_id), followers_count DESC NULLS LAST, sc_synced_at DESC, sc_user_id DESC LIMIT $2
OFFSET $3
