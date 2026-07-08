SELECT external_id
FROM wanted_tracks
WHERE id = $1
  AND source = 'genius_crawl'
