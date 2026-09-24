SELECT urn,
       sc_playlist_id,
       title,
       title_normalized,
       description,
       genre,
       tags,
       artwork_url,
       permalink_url,
       owner_sc_user_id,
       owner_urn,
       owner_username,
       track_count,
       duration_ms,
       playlist_type,
       kind,
       sharing,
       sc_metadata,
       deleted_at,
       release_year,
       release_date,
       label_name,
       likes_count_sc,
       reposts_count_sc,
       sc_created_at,
       sc_last_modified,
       sc_synced_at,
       last_read_at,
       created_at,
       updated_at
FROM playlists
WHERE sharing = 'public'
  AND deleted_at IS NULL
  AND ($1::text IS NULL OR
    title_normalized LIKE $4
        OR (NOT $5::bool AND (LOWER(title) LIKE $1
        OR LOWER(owner_username) LIKE $1))
    )
ORDER BY likes_count_sc DESC NULLS LAST, sc_synced_at DESC, urn DESC LIMIT $2
OFFSET $3
