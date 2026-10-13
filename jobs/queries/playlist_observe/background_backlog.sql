SELECT count(*) AS "waiting!"
FROM (
    SELECT 1
    FROM background_jobs
    WHERE kind = 'playlists.observe_shadow'
      AND priority = 0
      AND lease_id IS NULL
    LIMIT $1
) AS waiting
