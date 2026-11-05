SELECT login_request_id AS "login_request_id!",
       state AS "state!",
       code_challenge AS "code_challenge!",
       client_id AS "client_id!",
       redirect_uri AS "redirect_uri!"
FROM public.auth_login_create($1, $2, $3)
