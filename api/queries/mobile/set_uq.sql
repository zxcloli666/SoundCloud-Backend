UPDATE mobile_credentials
SET uq_opt_in = $2
WHERE soundcloud_user_id = $1
RETURNING soundcloud_user_id
