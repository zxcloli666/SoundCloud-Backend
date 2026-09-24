INSERT INTO artists (name, normalized_name, mb_artist_id, genius_artist_id, source, confidence)
VALUES ($1, $2, $3, $4, 'crawl', 0.7)
ON CONFLICT DO NOTHING
RETURNING id
