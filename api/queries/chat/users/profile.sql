SELECT username, avatar_url, permalink_url, verified
FROM public.users
WHERE sc_user_id = $1
