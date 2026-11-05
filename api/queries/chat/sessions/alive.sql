SELECT alive AS "alive!"
FROM public.auth_sessions_alive($1::bytea[]) AS alive
