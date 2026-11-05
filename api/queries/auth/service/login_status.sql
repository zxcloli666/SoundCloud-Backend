SELECT status AS "status!",
       step,
       username,
       result_session_id,
       error,
       redirect_url,
       profile_ok,
       chat_bind
FROM public.auth_login_status($1)
