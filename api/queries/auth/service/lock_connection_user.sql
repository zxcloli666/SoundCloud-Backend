SELECT soundcloud_user_id,
       public.scd_hlc(token_v) AS "login_v!"
FROM soundcloud_connections
WHERE id = $1
FOR UPDATE
