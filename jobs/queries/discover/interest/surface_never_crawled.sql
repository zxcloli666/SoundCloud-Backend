UPDATE artists
SET genius_next_run_at = now()
FROM artist_interest AS interest
WHERE artists.id = interest.artist_id
  AND artists.merged_into IS NULL
  AND NOT artists.crawl_dead
  AND artists.genius_artist_id IS NOT NULL
  AND artists.genius_crawled_at IS NULL
  AND artists.genius_next_run_at > now()
