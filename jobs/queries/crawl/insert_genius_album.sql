INSERT INTO albums (title, normalized_title, primary_artist_id, type, release_year, genius_album_id,
                    cover_url, source, confidence)
VALUES ($1, $2, $3, 'album', $4, $5, $6, 'genius_crawl', 0.7)
ON CONFLICT DO NOTHING
RETURNING id
