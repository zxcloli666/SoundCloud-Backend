UPDATE tracks
SET index_priority = LEAST(index_priority, $2),
    storage_priority = LEAST(storage_priority, $3)
WHERE sc_track_id = $1
  AND (index_priority > $2 OR storage_priority > $3)
