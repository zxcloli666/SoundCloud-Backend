SELECT status,
       expires_at,
       plan,
       plan_seen_at,
       uq_opt_in
FROM mobile_credentials
WHERE soundcloud_user_id = $1
