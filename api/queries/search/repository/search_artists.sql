SELECT id,
       name,
       country,
       avatar_url,
       confidence,
       track_count_primary,
       track_count_featured,
       album_count_denorm,
       monthly_listeners,
       trending_score,
       tags,
       is_star,
       star_aura_id,
       star_custom_hex
FROM artists
WHERE merged_into IS NULL
  AND (track_count_primary > 0 OR track_count_featured > 0)
  AND (normalized_name LIKE $4 OR (NOT $5::bool AND LOWER(name) LIKE $1))
ORDER BY monthly_listeners DESC, trending_score DESC, normalized_name ASC, id ASC LIMIT $2
OFFSET $3
