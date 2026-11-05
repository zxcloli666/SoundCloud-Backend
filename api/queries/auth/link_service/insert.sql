SELECT link_request_id AS "link_request_id!",
       claim_token AS "claim_token!",
       expires_at AS "expires_at!"
FROM public.auth_link_create($1, $2, $3)
