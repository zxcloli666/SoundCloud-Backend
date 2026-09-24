SELECT profiles, synced_at >= now() - interval '1 day' AND synced_at <= now() AS "fresh!"
FROM user_web_profiles
WHERE sc_user_id = $1
