SELECT status AS "status!",
       mode AS "mode!",
       target_session_id,
       error,
       chat_bind
FROM public.auth_link_status($1)
