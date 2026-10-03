SELECT requested.sc_track_id AS "requested!",
       serving.sc_track_id AS "serving?"
FROM tracks requested
LEFT JOIN tracks serving
       ON serving.id = coalesce(requested.superseded_by, requested.id)
      AND serving.sharing = 'public'
      AND serving.deleted_at IS NULL
      AND serving.superseded_by IS NULL
WHERE requested.sc_track_id = ANY($1)
