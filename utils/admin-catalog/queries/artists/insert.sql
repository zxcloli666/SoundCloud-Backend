INSERT INTO artists (name, normalized_name, country, bio, avatar_url, sc_user_id, source, confidence)
VALUES ($1, $2, $3, $4, $5, $6, 'manual', 1.0)
ON CONFLICT (normalized_name) WHERE merged_into IS NULL DO NOTHING
RETURNING id, name, normalized_name, country, avatar_url, bio, sc_user_id, source,
          confidence, mb_artist_id, spotify_artist_id, genius_artist_id, merged_into, created_at, updated_at
