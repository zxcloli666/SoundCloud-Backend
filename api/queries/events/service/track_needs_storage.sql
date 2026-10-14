SELECT EXISTS (
    SELECT 1
    FROM tracks
    WHERE sc_track_id = $1
      AND storage_state NOT IN ('ok', 'too_long')
) AS "needs_storage!"
