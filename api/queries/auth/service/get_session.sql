SELECT soundcloud_connection_id,
       soundcloud_user_id AS "soundcloud_user_id?"
FROM public.auth_session($1)
