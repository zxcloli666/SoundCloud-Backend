UPDATE mobile_credentials
SET plan = $2, plan_seen_at = now()
WHERE soundcloud_user_id = $1 AND plan IS DISTINCT FROM $2
