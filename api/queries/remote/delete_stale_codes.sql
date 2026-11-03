DELETE FROM remote_link_codes
WHERE expires_at < now()
   OR (sc_user_id = $1 AND provider = $2)
