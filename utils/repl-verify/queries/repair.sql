SELECT r.peer AS "peer!", r.rel AS "rel!", r.k::text AS "k", r.action AS "action!", r.attempt AS "attempt!"
FROM public.scd_verify_repair($1, $2) AS r
