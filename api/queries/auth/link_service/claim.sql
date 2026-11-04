SELECT outcome AS "outcome!",
       session_id,
       mode
FROM public.auth_link_claim($1, $2, $3, $4)
