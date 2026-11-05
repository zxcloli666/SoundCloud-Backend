DELETE FROM remote_link_codes
WHERE id = $1 AND expires_at > now()
RETURNING id
