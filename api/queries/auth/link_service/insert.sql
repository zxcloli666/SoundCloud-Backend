SELECT link_request_id AS "link_request_id!",
       expires_at AS "expires_at!"
FROM public.auth_link_create($1, $2, $3, $4)
