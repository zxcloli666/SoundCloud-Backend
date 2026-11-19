SELECT public.scd_hlc(token_v) AS "link_v!"
FROM mobile_credentials
WHERE soundcloud_user_id = $1
FOR UPDATE
