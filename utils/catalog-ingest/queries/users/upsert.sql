INSERT INTO users (
   sc_user_id, urn, username, username_normalized, full_name, first_name, last_name,
   permalink, permalink_url, avatar_url, country, city, description, verified,
   followers_count, followings_count, tracks_count, playlists_count,
   reposts_count, comments_count, kind, sc_created_at, sc_last_modified, sc_synced_at, sc_observation
) VALUES (
   $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21,$22,$23,now(),$24
)
ON CONFLICT (sc_user_id) DO UPDATE SET
   urn = EXCLUDED.urn,
   username = EXCLUDED.username,
   username_normalized = EXCLUDED.username_normalized,
   full_name = CASE WHEN $25::jsonb ? 'full_name' THEN EXCLUDED.full_name ELSE users.full_name END,
   first_name = CASE WHEN $25::jsonb ? 'first_name' THEN EXCLUDED.first_name ELSE users.first_name END,
   last_name = CASE WHEN $25::jsonb ? 'last_name' THEN EXCLUDED.last_name ELSE users.last_name END,
   permalink = CASE WHEN $25::jsonb ? 'permalink' THEN EXCLUDED.permalink ELSE users.permalink END,
   permalink_url = CASE WHEN $25::jsonb ? 'permalink_url' THEN EXCLUDED.permalink_url ELSE users.permalink_url END,
   avatar_url = CASE WHEN $25::jsonb ? 'avatar_url' THEN EXCLUDED.avatar_url ELSE users.avatar_url END,
   country = CASE WHEN $25::jsonb ?| ARRAY['country', 'country_code'] THEN EXCLUDED.country ELSE users.country END,
   city = CASE WHEN $25::jsonb ? 'city' THEN EXCLUDED.city ELSE users.city END,
   description = CASE WHEN $25::jsonb ? 'description' THEN EXCLUDED.description ELSE users.description END,
   verified = CASE WHEN $25::jsonb ? 'verified' THEN EXCLUDED.verified ELSE users.verified END,
   followers_count = COALESCE(EXCLUDED.followers_count, users.followers_count),
   followings_count = COALESCE(EXCLUDED.followings_count, users.followings_count),
   tracks_count = COALESCE(EXCLUDED.tracks_count, users.tracks_count),
   playlists_count = COALESCE(EXCLUDED.playlists_count, users.playlists_count),
   reposts_count = COALESCE(EXCLUDED.reposts_count, users.reposts_count),
   comments_count = COALESCE(EXCLUDED.comments_count, users.comments_count),
   kind = CASE WHEN $25::jsonb ? 'kind' THEN EXCLUDED.kind ELSE users.kind END,
   sc_created_at = COALESCE(EXCLUDED.sc_created_at, users.sc_created_at),
   sc_last_modified = COALESCE(EXCLUDED.sc_last_modified, users.sc_last_modified),
   sc_synced_at = now(),
   sc_observation = GREATEST(users.sc_observation, EXCLUDED.sc_observation),
   updated_at = now()
WHERE EXCLUDED.sc_observation > 0
  AND (EXCLUDED.sc_observation > users.sc_observation
       OR EXCLUDED.sc_last_modified > users.sc_last_modified)
  AND (users.sc_last_modified IS NULL
       OR COALESCE(EXCLUDED.sc_last_modified, users.sc_last_modified) >= users.sc_last_modified)
RETURNING (xmax = 0) AS "was_new!"
