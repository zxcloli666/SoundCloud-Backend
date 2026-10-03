SELECT id,
       normalized_name
FROM artists
WHERE normalized_name = ANY ($1)
  AND merged_into IS NULL
  AND (track_count_primary > 0 OR track_count_featured > 0)
