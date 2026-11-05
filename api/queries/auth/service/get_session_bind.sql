SELECT soundcloud_user_id AS "soundcloud_user_id!",
       chat_bind
FROM public.auth_session($1)
WHERE soundcloud_user_id IS NOT NULL
