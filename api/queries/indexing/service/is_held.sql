SELECT EXISTS (
    SELECT 1
    FROM tracks
    WHERE sc_track_id = $1
      AND pipeline_held
      AND deleted_at IS NULL
) AS "held!"
