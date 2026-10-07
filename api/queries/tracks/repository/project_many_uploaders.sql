SELECT sc_user_id,
       jsonb_build_object(
               'kind', 'user',
               'id', CASE
                   WHEN sc_user_id ~ '^[1-9][0-9]{0,17}$' THEN to_jsonb(sc_user_id::bigint)
                   ELSE to_jsonb(sc_user_id)
               END,
               'urn', urn,
               'username', username,
               'avatar_url', avatar_url,
               'permalink_url', permalink_url,
               'verified', verified,
               'country_code', country,
               'city', city,
               'description', description,
               'followers_count', followers_count,
               'followings_count', followings_count,
               'track_count', tracks_count
       ) AS "u!"
FROM users
WHERE sc_user_id = ANY ($1)
