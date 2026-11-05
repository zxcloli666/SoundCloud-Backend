DELETE FROM remote_link_codes
WHERE sc_user_id = $1 AND provider = $2
