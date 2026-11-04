SELECT p.profile_json || CASE
           WHEN u.sc_synced_at > p.synced_at THEN jsonb_strip_nulls(jsonb_build_object(
               'username', u.username,
               'full_name', u.full_name,
               'first_name', u.first_name,
               'last_name', u.last_name,
               'avatar_url', u.avatar_url,
               'permalink', u.permalink,
               'permalink_url', u.permalink_url,
               'city', u.city,
               'country_code', u.country,
               'description', u.description,
               'verified', u.verified,
               'followers_count', u.followers_count,
               'followings_count', u.followings_count
           ))
           ELSE '{}'::jsonb
       END AS "profile_json!",
       p.synced_at
FROM user_profiles p
LEFT JOIN LATERAL (
    SELECT *
    FROM users
    WHERE sc_user_id = ANY ($1)
    ORDER BY sc_synced_at DESC
    LIMIT 1
) u ON true
WHERE p.soundcloud_user_id = ANY ($1)
ORDER BY p.synced_at DESC
LIMIT 1
