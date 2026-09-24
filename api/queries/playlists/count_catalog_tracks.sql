SELECT count(*) AS "count!"
FROM tracks
WHERE sc_track_id = ANY ($1::text[])
  AND deleted_at IS NULL
