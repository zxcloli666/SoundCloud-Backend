INSERT INTO albums (title, normalized_title, primary_artist_id, type, release_year, mb_release_id,
                    source, confidence)
VALUES ($1, $2, $3, $4, $5, $6, 'mb_crawl', 0.7)
ON CONFLICT DO NOTHING
RETURNING id
